use std::collections::HashSet;

use quote::ToTokens;
use syn::{
    BinOp, Expr, ExprArray, ExprBinary, ExprCall, ExprLit, ExprPath, ExprUnary, Lit, UnOp,
    spanned::Spanned,
};

const ENUM_MARKER_PREFIX: &str = "__orm_enum__";

pub fn render_predicate(expr: &Expr, columns: &HashSet<String>) -> syn::Result<String> {
    Renderer { columns }.predicate(expr)
}

pub fn predicate_can_be_unknown(
    expr: &Expr,
    nullable_columns: &HashSet<String>,
) -> syn::Result<bool> {
    let mut referenced = HashSet::new();
    collect_nullable_columns(expr, nullable_columns, &mut referenced);
    let referenced = referenced.into_iter()
        .collect::<Vec<_>>();

    if referenced.len() > 16 {
        return Err(syn::Error::new(
            expr.span(),
            "a database validation references too many nullable columns to prove its SQL NULL behavior",
        ));
    }

    for mask in 0..(1_u64 << referenced.len()) {
        let null_columns = referenced
            .iter()
            .enumerate()
            .filter_map(|(index, column)| ((mask & (1 << index)) != 0).then_some(*column))
            .collect::<HashSet<_>>();

        if (TruthEvaluator { null_columns }.truth(expr)?)
            .contains(TruthSet::UNKNOWN)
        {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn render_default(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => render_literal(lit),
        Expr::Unary(ExprUnary { op: UnOp::Neg(_), expr, .. }) => match expr.as_ref() {
            Expr::Lit(ExprLit { lit: Lit::Int(value), .. }) => Ok(format!("-{}", value.base10_digits())),
            Expr::Lit(ExprLit { lit: Lit::Float(value), .. }) => Ok(format!("-{}", value.base10_digits())),
            _ => Err(syn::Error::new(expr.span(), "a negative default must contain a numeric literal")),
        },
        Expr::Call(call) => {
            let name = call_name(call)?;
            if !call.args.is_empty() {
                return Err(syn::Error::new(call.span(), "database default generators do not take arguments"));
            }

            match name.as_str() {
                "now" | "gen_random_uuid" => Ok(format!("{name}()")),
                _ => Err(syn::Error::new(
                    call.span(),
                    "unsupported database default generator; expected now() or gen_random_uuid()",
                )),
            }
        }
        Expr::Path(path) => render_enum_path(path),
        _ => Err(syn::Error::new(
            expr.span(),
            "unsupported database default; use a scalar literal, now(), gen_random_uuid(), or a registered enum variant",
        )),
    }
}

struct Renderer<'a> {
    columns: &'a HashSet<String>,
}

#[derive(Clone, Copy)]
struct TruthSet(u8);

impl TruthSet {
    const TRUE: u8 = 1;
    const FALSE: u8 = 2;
    const UNKNOWN: u8 = 4;

    fn exact(value: u8) -> Self {
        Self(value)
    }

    fn possible_boolean() -> Self {
        Self(Self::TRUE | Self::FALSE)
    }

    fn contains(self, value: u8) -> bool {
        self.0 & value != 0
    }

    fn values(self) -> impl Iterator<Item = u8> {
        [Self::TRUE, Self::FALSE, Self::UNKNOWN]
            .into_iter()
            .filter(move |value| self.contains(*value))
    }

    fn combine(
        self,
        other: Self,
        operation: impl Fn(u8, u8) -> u8,
    ) -> Self {
        let mut values = 0;
        for left in self.values() {
            for right in other.values() {
                values |= operation(left, right);
            }
        }

        Self(values)
    }

    fn not(self) -> Self {
        let mut values = 0;
        for value in self.values() {
            values |= match value {
                Self::TRUE => Self::FALSE,
                Self::FALSE => Self::TRUE,
                _ => Self::UNKNOWN,
            };
        }

        Self(values)
    }
}

#[derive(Clone, Copy)]
enum NullState {
    Null,
    NonNull,
    Maybe,
}

struct TruthEvaluator<'a> {
    null_columns: HashSet<&'a str>,
}

impl TruthEvaluator<'_> {
    fn truth(&self, expr: &Expr) -> syn::Result<TruthSet> {
        let expr = strip_grouping(expr);
        match expr {
            Expr::Binary(binary) => self.binary_truth(binary),
            Expr::Unary(unary) => match &unary.op {
                UnOp::Not(_) => Ok(
                    self.truth(&unary.expr)?
                        .not()
                ),
                UnOp::Neg(_) => Ok(self.scalar_truth(expr)),
                _ => Err(syn::Error::new(unary.op.span(), "unsupported PostgreSQL predicate operator")),
            },
            Expr::Call(call) => self.call_truth(call),
            Expr::Path(path) => {
                if path.path.segments.len() > 1 {
                    return Ok(TruthSet::possible_boolean());
                }

                Ok(self.scalar_truth(expr))
            }
            Expr::Lit(ExprLit { lit: Lit::Bool(value), .. }) => {
                Ok(TruthSet::exact(if value.value { TruthSet::TRUE } else { TruthSet::FALSE }))
            }
            Expr::Lit(_) => Ok(TruthSet::possible_boolean()),
            _ => Err(syn::Error::new(expr.span(), "unsupported PostgreSQL predicate expression")),
        }
    }

    fn binary_truth(&self, binary: &ExprBinary) -> syn::Result<TruthSet> {
        match &binary.op {
            BinOp::And(_) => Ok(
                self.truth(&binary.left)?
                    .combine(self.truth(&binary.right)?, sql_and)
            ),
            BinOp::Or(_) => Ok(
                self.truth(&binary.left)?
                    .combine(self.truth(&binary.right)?, sql_or)
            ),
            BinOp::Eq(_) | BinOp::Ne(_) if is_boolean_expression(&binary.left) || is_boolean_expression(&binary.right) => {
                let equal = self.truth(&binary.left)?
                    .combine(self.truth(&binary.right)?, sql_equal);

                if matches!(&binary.op, BinOp::Ne(_)) {
                    Ok(equal.not())
                } else {
                    Ok(equal)
                }
            }
            BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) => {
                Ok(comparison_truth(
                    self.null_state(&binary.left),
                    self.null_state(&binary.right),
                ))
            }
            BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_) => {
                Ok(truth_from_null_state(merge_null_state(
                    self.null_state(&binary.left),
                    self.null_state(&binary.right),
                )))
            }
            _ => Err(syn::Error::new(binary.op.span(), "unsupported PostgreSQL predicate operator")),
        }
    }

    fn call_truth(&self, call: &ExprCall) -> syn::Result<TruthSet> {
        let name = call_name(call)?;
        let args = call.args.iter()
            .collect::<Vec<_>>();

        match name.as_str() {
            "is_null" | "is_not_null" => {
                expect_args(call, &args, 1)?;
                let result = match self.null_state(args[0]) {
                    NullState::Null => TruthSet::exact(TruthSet::TRUE),
                    NullState::NonNull => TruthSet::exact(TruthSet::FALSE),
                    NullState::Maybe => TruthSet::possible_boolean(),
                };

                if name == "is_not_null" {
                    Ok(result.not())
                } else {
                    Ok(result)
                }
            }
            "one_of" | "between" | "matches" => {
                let expected = if name == "between" { 3 } else { 2 };
                expect_args(call, &args, expected)?;
                let mut state = self.null_state(args[0]);
                for argument in args.iter()
                    .skip(1)
                {
                    if let Expr::Array(array) = argument {
                        for value in &array.elems {
                            state = merge_null_state(state, self.null_state(value));
                        }
                    } else {
                        state = merge_null_state(state, self.null_state(argument));
                    }
                }

                Ok(match state {
                    NullState::Null => TruthSet::exact(TruthSet::UNKNOWN),
                    NullState::NonNull => TruthSet::possible_boolean(),
                    NullState::Maybe => TruthSet(TruthSet::TRUE | TruthSet::FALSE | TruthSet::UNKNOWN),
                })
            }
            "exactly_one" => Ok(TruthSet::possible_boolean()),
            "implies" => {
                expect_args(call, &args, 2)?;
                Ok(
                    self.truth(args[0])?
                        .not()
                        .combine(self.truth(args[1])?, sql_or)
                )
            }
            "present_iff" => {
                expect_args(call, &args, 2)?;
                let present = match self.null_state(args[0]) {
                    NullState::Null => TruthSet::exact(TruthSet::FALSE),
                    NullState::NonNull => TruthSet::exact(TruthSet::TRUE),
                    NullState::Maybe => TruthSet::possible_boolean(),
                };

                Ok(present.combine(self.truth(args[1])?, sql_equal))
            }
            "required_if" => {
                expect_args(call, &args, 2)?;
                let present = match self.null_state(args[0]) {
                    NullState::Null => TruthSet::exact(TruthSet::FALSE),
                    NullState::NonNull => TruthSet::exact(TruthSet::TRUE),
                    NullState::Maybe => TruthSet::possible_boolean(),
                };

                Ok(
                    self.truth(args[1])?
                        .not()
                        .combine(present, sql_or)
                )
            }
            "length" | "char_length" | "trim" | "btrim" | "abs" | "num_nonnulls" => {
                let state = if name == "num_nonnulls" {
                    NullState::NonNull
                } else {
                    call.args
                        .iter()
                        .map(|argument| self.null_state(argument))
                        .fold(NullState::NonNull, merge_null_state)
                };

                Ok(truth_from_null_state(state))
            }
            _ => Err(syn::Error::new(call.span(), format!("unsupported PostgreSQL predicate function `{name}`"))),
        }
    }

    fn scalar_truth(&self, expr: &Expr) -> TruthSet {
        truth_from_null_state(self.null_state(expr))
    }

    fn null_state(&self, expr: &Expr) -> NullState {
        let expr = strip_grouping(expr);
        match expr {
            Expr::Path(path) if path.path.segments.len() == 1 => {
                let name = path.path.segments[0].ident.to_string();
                if self.null_columns.contains(name.as_str()) {
                    NullState::Null
                } else {
                    NullState::NonNull
                }
            }
            Expr::Path(_) | Expr::Lit(_) => NullState::NonNull,
            Expr::Unary(unary) => self.null_state(&unary.expr),
            Expr::Binary(binary) if matches!(
                &binary.op,
                BinOp::Add(_) | BinOp::Sub(_) | BinOp::Mul(_) | BinOp::Div(_)
            ) => merge_null_state(
                self.null_state(&binary.left),
                self.null_state(&binary.right),
            ),
            Expr::Binary(_) => truth_null_state(self.truth(expr)),
            Expr::Call(call) => {
                let Ok(name) = call_name(call) else {
                    return NullState::Maybe;
                };

                if name == "num_nonnulls" || matches!(
                    name.as_str(),
                    "is_null" | "is_not_null" | "exactly_one"
                ) {
                    return NullState::NonNull;
                }

                if matches!(
                    name.as_str(),
                    "one_of" | "implies" | "present_iff" | "required_if" | "between" | "matches"
                ) {
                    return truth_null_state(self.truth(expr));
                }

                call.args
                    .iter()
                    .map(|argument| self.null_state(argument))
                    .fold(NullState::NonNull, merge_null_state)
            }
            _ => NullState::Maybe,
        }
    }
}

fn collect_nullable_columns<'a>(
    expression: &Expr,
    nullable_columns: &'a HashSet<String>,
    found: &mut HashSet<&'a str>,
) {
    match expression {
        Expr::Path(path) if path.path.segments.len() == 1 => {
            let name = path.path.segments[0].ident.to_string();
            if let Some(column) = nullable_columns.get(&name) {
                found.insert(column);
            }
        }
        Expr::Call(call) => {
            for argument in &call.args {
                collect_nullable_columns(argument, nullable_columns, found);
            }
        }
        Expr::Binary(binary) => {
            collect_nullable_columns(&binary.left, nullable_columns, found);
            collect_nullable_columns(&binary.right, nullable_columns, found);
        }
        Expr::Unary(unary) => collect_nullable_columns(&unary.expr, nullable_columns, found),
        Expr::Paren(paren) => collect_nullable_columns(&paren.expr, nullable_columns, found),
        Expr::Group(group) => collect_nullable_columns(&group.expr, nullable_columns, found),
        Expr::Array(array) => {
            for element in &array.elems {
                collect_nullable_columns(element, nullable_columns, found);
            }
        }
        _ => {}
    }
}

fn is_boolean_expression(expr: &Expr) -> bool {
    match strip_grouping(expr) {
        Expr::Binary(binary) => matches!(
            &binary.op,
            BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) | BinOp::And(_) | BinOp::Or(_)
        ),
        Expr::Unary(unary) => matches!(&unary.op, UnOp::Not(_)),
        Expr::Call(call) => call_name(call)
            .map(|name| matches!(
                name.as_str(),
                "is_null" | "is_not_null" | "one_of" | "exactly_one" | "implies" | "present_iff" | "required_if" | "between" | "matches"
            ))
            .unwrap_or(false),
        Expr::Lit(ExprLit { lit: Lit::Bool(_), .. }) => true,
        _ => false,
    }
}

fn truth_null_state(result: syn::Result<TruthSet>) -> NullState {
    match result {
        Ok(values) if values.contains(TruthSet::UNKNOWN)
            && (values.contains(TruthSet::TRUE) || values.contains(TruthSet::FALSE)) => NullState::Maybe,
        Ok(values) if values.contains(TruthSet::UNKNOWN) => NullState::Null,
        Ok(_) => NullState::NonNull,
        Err(_) => NullState::Maybe,
    }
}

fn merge_null_state(left: NullState, right: NullState) -> NullState {
    match (left, right) {
        (NullState::Null, _) | (_, NullState::Null) => NullState::Null,
        (NullState::Maybe, _) | (_, NullState::Maybe) => NullState::Maybe,
        _ => NullState::NonNull,
    }
}

fn comparison_truth(left: NullState, right: NullState) -> TruthSet {
    truth_from_null_state(merge_null_state(left, right))
}

fn truth_from_null_state(state: NullState) -> TruthSet {
    match state {
        NullState::Null => TruthSet::exact(TruthSet::UNKNOWN),
        NullState::NonNull => TruthSet::possible_boolean(),
        NullState::Maybe => TruthSet(TruthSet::TRUE | TruthSet::FALSE | TruthSet::UNKNOWN),
    }
}

fn sql_and(left: u8, right: u8) -> u8 {
    if left == TruthSet::FALSE || right == TruthSet::FALSE {
        TruthSet::FALSE
    } else if left == TruthSet::UNKNOWN || right == TruthSet::UNKNOWN {
        TruthSet::UNKNOWN
    } else {
        TruthSet::TRUE
    }
}

fn sql_or(left: u8, right: u8) -> u8 {
    if left == TruthSet::TRUE || right == TruthSet::TRUE {
        TruthSet::TRUE
    } else if left == TruthSet::UNKNOWN || right == TruthSet::UNKNOWN {
        TruthSet::UNKNOWN
    } else {
        TruthSet::FALSE
    }
}

fn sql_equal(left: u8, right: u8) -> u8 {
    if left == TruthSet::UNKNOWN || right == TruthSet::UNKNOWN {
        TruthSet::UNKNOWN
    } else if left == right {
        TruthSet::TRUE
    } else {
        TruthSet::FALSE
    }
}

impl Renderer<'_> {
    fn predicate(&self, expr: &Expr) -> syn::Result<String> {
        self.render(expr, 0, false)
    }

    fn render(
        &self,
        expr: &Expr,
        parent_precedence: u8,
        right_operand: bool,
    ) -> syn::Result<String> {
        let expr = strip_grouping(expr);
        let precedence = self.precedence(expr)?;
        let rendered = self.render_unwrapped(expr)?;
        if precedence < parent_precedence || (right_operand && precedence == parent_precedence) {
            Ok(format!("({rendered})"))
        } else {
            Ok(rendered)
        }
    }

    fn render_unwrapped(&self, expr: &Expr) -> syn::Result<String> {
        match expr {
            Expr::Binary(binary) => self.binary(binary),
            Expr::Unary(unary) => self.unary(unary),
            Expr::Call(call) => self.call(call),
            Expr::Path(path) => self.path(path),
            Expr::Lit(ExprLit { lit, .. }) => render_literal(lit),
            _ => Err(syn::Error::new(
                expr.span(),
                format!("unsupported PostgreSQL predicate expression `{}`", expr.to_token_stream()),
            )),
        }
    }

    fn precedence(&self, expr: &Expr) -> syn::Result<u8> {
        match strip_grouping(expr) {
            Expr::Binary(binary) => binary_precedence(&binary.op),
            Expr::Unary(_) => Ok(6),
            Expr::Call(call) => match call_name(call)?
                .as_str()
            {
                "implies" | "required_if" => Ok(1),
                "is_null" | "is_not_null" | "one_of" | "exactly_one" | "present_iff" | "between" | "matches" => Ok(3),
                _ => Ok(7),
            },
            Expr::Path(_) | Expr::Lit(_) => Ok(7),
            other => Err(syn::Error::new(other.span(), "unsupported PostgreSQL predicate expression")),
        }
    }

    fn binary(&self, binary: &ExprBinary) -> syn::Result<String> {
        let operator = match &binary.op {
            BinOp::Eq(_) => "=",
            BinOp::Ne(_) => "<>",
            BinOp::Lt(_) => "<",
            BinOp::Le(_) => "<=",
            BinOp::Gt(_) => ">",
            BinOp::Ge(_) => ">=",
            BinOp::And(_) => "AND",
            BinOp::Or(_) => "OR",
            BinOp::Add(_) => "+",
            BinOp::Sub(_) => "-",
            BinOp::Mul(_) => "*",
            BinOp::Div(_) => "/",
            _ => {
                return Err(syn::Error::new(
                    binary.op.span(),
                    "unsupported PostgreSQL predicate operator",
                ));
            }
        };

        let precedence = binary_precedence(&binary.op)?;
        let left = self.binary_operand(&binary.left, precedence, false)?;
        let right = self.binary_operand(&binary.right, precedence, true)?;
        Ok(format!(
            "{} {operator} {}",
            left,
            right,
        ))
    }

    fn binary_operand(
        &self,
        expr: &Expr,
        parent_precedence: u8,
        right_operand: bool,
    ) -> syn::Result<String> {
        let expr = strip_grouping(expr);
        let child_precedence = self.precedence(expr)?;
        let force_group = (parent_precedence == 1 && child_precedence == 2)
            || (parent_precedence == 1 && is_comparison_of_comparisons(expr, self)?)
            || (parent_precedence == 3 && child_precedence == 3);

        if force_group {
            Ok(format!("({})", self.render(expr, 0, false)?))
        } else {
            self.render(expr, parent_precedence, right_operand)
        }
    }

    fn unary(&self, unary: &ExprUnary) -> syn::Result<String> {
        match &unary.op {
            UnOp::Not(_) => Ok(format!("NOT {}", self.render(&unary.expr, 6, true)?)),
            UnOp::Neg(_) => Ok(format!("-{}", self.render(&unary.expr, 6, true)?)),
            _ => Err(syn::Error::new(unary.op.span(), "unsupported PostgreSQL predicate operator")),
        }
    }

    fn path(&self, path: &ExprPath) -> syn::Result<String> {
        if path.path.segments.len() == 1 {
            let name = path.path.segments[0].ident.to_string();
            if self.columns.contains(&name) {
                return Ok(name);
            }

            return Err(syn::Error::new(path.span(), format!("unknown table column `{name}`")));
        }

        render_enum_path(path)
    }

    fn call(&self, call: &ExprCall) -> syn::Result<String> {
        let name = call_name(call)?;
        let args = call.args.iter()
            .collect::<Vec<_>>();

        match name.as_str() {
            "is_null" | "is_not_null" => {
                expect_args(call, &args, 1)?;
                let operator = if name == "is_null" { "IS NULL" } else { "IS NOT NULL" };
                Ok(format!("{} {operator}", self.render(args[0], 3, false)?))
            }
            "one_of" => {
                expect_args(call, &args, 2)?;
                let Expr::Array(ExprArray { elems, .. }) = args[1] else {
                    return Err(syn::Error::new(args[1].span(), "one_of expects an array as its second argument"));
                };

                if elems.is_empty() {
                    return Err(syn::Error::new(args[1].span(), "one_of expects at least one value"));
                }

                let values = elems
                    .iter()
                    .map(|value| self.render(value, 0, false))
                    .collect::<syn::Result<Vec<_>>>()?;

                Ok(format!("{} IN ({})", self.render(args[0], 3, false)?, values.join(", ")))
            }
            "exactly_one" => {
                if args.len() < 2 {
                    return Err(syn::Error::new(call.span(), "exactly_one expects at least two columns"));
                }

                let columns = args
                    .iter()
                    .map(|value| self.render(value, 0, false))
                    .collect::<syn::Result<Vec<_>>>()?;

                Ok(format!("num_nonnulls({}) = 1", columns.join(", ")))
            }
            "implies" => {
                expect_args(call, &args, 2)?;
                Ok(format!(
                    "NOT ({}) OR {}",
                    self.render(args[0], 0, false)?,
                    self.render(args[1], 1, true)?,
                ))
            }
            "present_iff" => {
                expect_args(call, &args, 2)?;
                Ok(format!(
                    "({} IS NOT NULL) = ({})",
                    self.column_argument(args[0], "present_iff")?,
                    self.render(args[1], 0, false)?,
                ))
            }
            "required_if" => {
                expect_args(call, &args, 2)?;
                Ok(format!(
                    "NOT ({}) OR {} IS NOT NULL",
                    self.render(args[1], 0, false)?,
                    self.column_argument(args[0], "required_if")?,
                ))
            }
            "between" => {
                expect_args(call, &args, 3)?;
                Ok(format!(
                    "{} BETWEEN {} AND {}",
                    self.render(args[0], 3, false)?,
                    self.render(args[1], 0, false)?,
                    self.render(args[2], 0, false)?,
                ))
            }
            "matches" => {
                expect_args(call, &args, 2)?;
                Ok(format!(
                    "{} ~ {}",
                    self.render(args[0], 3, false)?,
                    self.render(args[1], 3, true)?,
                ))
            }
            "length" | "char_length" | "trim" | "btrim" | "abs" | "num_nonnulls" => {
                if args.is_empty() {
                    return Err(syn::Error::new(call.span(), format!("{name} expects at least one argument")));
                }

                let rendered = args
                    .iter()
                    .map(|value| self.render(value, 0, false))
                    .collect::<syn::Result<Vec<_>>>()?;

                Ok(format!("{name}({})", rendered.join(", ")))
            }
            _ => Err(syn::Error::new(call.span(), format!("unsupported PostgreSQL predicate function `{name}`"))),
        }
    }

    fn column_argument(&self, expr: &Expr, function: &str) -> syn::Result<String> {
        let Expr::Path(path) = expr else {
            return Err(syn::Error::new(expr.span(), format!("{function}'s first argument must be a nullable column")));
        };

        if path.path.segments.len() != 1 {
            return Err(syn::Error::new(expr.span(), format!("{function}'s first argument must be a nullable column")));
        }

        self.path(path)
    }
}

fn strip_grouping(mut expr: &Expr) -> &Expr {
    loop {
        match expr {
            Expr::Paren(paren) => expr = &paren.expr,
            Expr::Group(group) => expr = &group.expr,
            _ => return expr,
        }
    }
}

fn binary_precedence(operator: &BinOp) -> syn::Result<u8> {
    match operator {
        BinOp::Or(_) => Ok(1),
        BinOp::And(_) => Ok(2),
        BinOp::Eq(_) | BinOp::Ne(_) | BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) => Ok(3),
        BinOp::Add(_) | BinOp::Sub(_) => Ok(4),
        BinOp::Mul(_) | BinOp::Div(_) => Ok(5),
        _ => Err(syn::Error::new(operator.span(), "unsupported PostgreSQL predicate operator")),
    }
}

fn is_comparison_of_comparisons(expr: &Expr, renderer: &Renderer<'_>) -> syn::Result<bool> {
    let Expr::Binary(binary) = strip_grouping(expr) else {
        return Ok(false);
    };

    if binary_precedence(&binary.op)? != 3 {
        return Ok(false);
    }

    Ok(renderer.precedence(&binary.left)? == 3 || renderer.precedence(&binary.right)? == 3)
}

fn expect_args(call: &ExprCall, args: &[&Expr], expected: usize) -> syn::Result<()> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(syn::Error::new(
            call.span(),
            format!("{} expects {expected} arguments", call_name(call)?),
        ))
    }
}

fn call_name(call: &ExprCall) -> syn::Result<String> {
    let Expr::Path(path) = call.func.as_ref() else {
        return Err(syn::Error::new(call.func.span(), "expected a named schema function"));
    };

    if path.path.segments.len() != 1 {
        return Err(syn::Error::new(path.span(), "schema functions cannot be qualified"));
    }

    Ok(path.path.segments[0].ident.to_string())
}

fn render_enum_path(path: &ExprPath) -> syn::Result<String> {
    let segments = path.path.segments.iter()
        .collect::<Vec<_>>();

    if segments.len() != 2 {
        return Err(syn::Error::new(
            path.span(),
            "enum values must use `EnumType::Variant` syntax",
        ));
    }

    Ok(format!(
        "'{ENUM_MARKER_PREFIX}{}__{}'",
        segments[0].ident, segments[1].ident,
    ))
}

fn render_literal(literal: &Lit) -> syn::Result<String> {
    match literal {
        Lit::Str(value) => Ok(format!("'{}'", value.value().replace('\'', "''"))),
        Lit::Int(value) => Ok(
            value.base10_digits()
                .to_string()
        ),
        Lit::Float(value) => Ok(
            value.base10_digits()
                .to_string()
        ),
        Lit::Bool(value) => Ok(
            value.value()
                .to_string()
        ),
        _ => Err(syn::Error::new(literal.span(), "unsupported PostgreSQL scalar literal")),
    }
}

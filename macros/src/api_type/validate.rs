//! Shared codegen for orm-owned validation. Reads each field's
//! `#[api(validate(...))]` rules and generates `impl ::orm::validate::Validate`
//! whose body calls validator's runtime checks (re-exported by orm). Used by
//! `api_type`, `table_type`, and the generated Insert/Update structs so consumers
//! never depend on `validator` directly.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    Attribute, Fields, Generics, Ident, LitInt, LitStr, Token, Type, parenthesized,
    parse::ParseStream,
};

/// One validation rule from `#[api(validate(...))]`. Args use call syntax:
/// `length(min(3), max(30))`, `range(min(0))`, `regex(r"^\d+$")`.
enum Rule {
    Email,
    Length { min: Option<u64>, max: Option<u64>, equal: Option<u64> },
    Range { min: Option<LitInt>, max: Option<LitInt> },
    /// Inline pattern; compiled once into a per-field static `Regex`.
    Regex(LitStr),
    Required,
}

/// Generates `impl ::orm::validate::Validate for #name` from the fields'
/// `#[api(validate(...))]` rules (empty/always-`Ok` when there are none).
pub fn generate(name: &Ident, generics: &Generics, fields: &Fields) -> TokenStream {
    let mut checks = Vec::new();
    if let Fields::Named(named) = fields {
        for field in &named.named {
            let rules = match field_rules(&field.attrs) {
                Ok(rules) => rules,
                Err(err) => return err.to_compile_error(),
            };
            if rules.is_empty() {
                continue;
            }
            let ident = field.ident.clone().expect("named field");
            let field_str = ident.to_string();
            let optional = is_option(&field.ty);
            for rule in &rules {
                checks.push(rule_check(rule, &ident, &field_str, optional));
            }
        }
    }

    let (imp, ty, whr) = generics.split_for_impl();
    quote! {
        impl #imp ::orm::validate::Validate for #name #ty #whr {
            fn validate(&self) -> ::std::result::Result<(), ::orm::validate::ValidationErrors> {
                #[allow(unused_imports)]
                use ::orm::validate::{ValidateEmail, ValidateLength, ValidateRange, ValidateRegex};
                let mut __errors = ::orm::validate::ValidationErrors::new();
                #(#checks)*
                if __errors.is_empty() {
                    ::std::result::Result::Ok(())
                } else {
                    ::std::result::Result::Err(__errors)
                }
            }
        }
    }
}

/// Builds the check for one rule on `self.#ident` (wrapping `Option<_>` fields so
/// only the inner value is checked when present).
fn rule_check(rule: &Rule, ident: &Ident, field: &str, optional: bool) -> TokenStream {
    let recv = if optional { quote!(__inner) } else { quote!(self.#ident) };

    // Rules that don't fit the plain `if !<bool> { add }` shape.
    match rule {
        Rule::Required => {
            return if optional {
                let err = make_error("required", "is required");
                quote! { if self.#ident.is_none() { __errors.add(#field, #err); } }
            } else {
                quote! {} // a non-Option field is always present
            };
        }
        Rule::Regex(pattern) => {
            // Inline pattern compiled once into a block-local static.
            let err = make_error("regex", "has an invalid format");
            let check = quote! {
                {
                    static __RE: ::std::sync::LazyLock<::orm::validate::Regex> =
                        ::std::sync::LazyLock::new(|| {
                            ::orm::validate::Regex::new(#pattern)
                                .expect("invalid #[api(validate(regex(...)))] pattern")
                        });
                    if !#recv.validate_regex(&*__RE) {
                        __errors.add(#field, #err);
                    }
                }
            };
            return wrap_optional(check, ident, optional);
        }
        _ => {}
    }

    let (passed, code, message) = match rule {
        Rule::Email => {
            (quote! { #recv.validate_email() }, "email", "must be a valid email address".to_string())
        }
        Rule::Length { min, max, equal } => {
            let (min_t, max_t, equal_t) = (opt_u64(min), opt_u64(max), opt_u64(equal));
            (
                quote! { #recv.validate_length(#min_t, #max_t, #equal_t) },
                "length",
                length_message(min, max, equal),
            )
        }
        Rule::Range { min, max } => {
            let (min_t, max_t) = (opt_lit(min), opt_lit(max));
            (quote! { #recv.validate_range(#min_t, #max_t, None, None) }, "range", range_message(min, max))
        }
        Rule::Regex(_) | Rule::Required => unreachable!(),
    };

    let err = make_error(code, &message);
    wrap_optional(quote! { if !#passed { __errors.add(#field, #err); } }, ident, optional)
}

fn wrap_optional(check: TokenStream, ident: &Ident, optional: bool) -> TokenStream {
    if optional {
        quote! { if let ::std::option::Option::Some(__inner) = &self.#ident { #check } }
    } else {
        check
    }
}

fn make_error(code: &str, message: &str) -> TokenStream {
    quote! {
        {
            let mut __err = ::orm::validate::ValidationError::new(#code);
            __err.message = ::std::option::Option::Some(::std::borrow::Cow::Borrowed(#message));
            __err
        }
    }
}

fn opt_u64(value: &Option<u64>) -> TokenStream {
    match value {
        Some(n) => quote! { ::std::option::Option::Some(#n) },
        None => quote! { ::std::option::Option::None },
    }
}

fn opt_lit(value: &Option<LitInt>) -> TokenStream {
    match value {
        Some(lit) => quote! { ::std::option::Option::Some(#lit) },
        None => quote! { ::std::option::Option::None },
    }
}

fn length_message(min: &Option<u64>, max: &Option<u64>, equal: &Option<u64>) -> String {
    match (min, max, equal) {
        (_, _, Some(e)) => format!("must be exactly {e} characters"),
        (Some(lo), Some(hi), _) => format!("must be between {lo} and {hi} characters"),
        (Some(lo), None, _) => format!("must be at least {lo} characters"),
        (None, Some(hi), _) => format!("must be at most {hi} characters"),
        _ => "has an invalid length".to_string(),
    }
}

fn range_message(min: &Option<LitInt>, max: &Option<LitInt>) -> String {
    let s = |l: &Option<LitInt>| l.as_ref().map(|v| v.base10_digits().to_string());
    match (s(min), s(max)) {
        (Some(lo), Some(hi)) => format!("must be between {lo} and {hi}"),
        (Some(lo), None) => format!("must be at least {lo}"),
        (None, Some(hi)) => format!("must be at most {hi}"),
        _ => "is out of range".to_string(),
    }
}

fn is_option(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "Option"))
}

/// Collects rules from every `#[api(validate(...))]` on a field.
fn field_rules(attrs: &[Attribute]) -> syn::Result<Vec<Rule>> {
    let mut rules = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("api") {
            continue;
        }
        attr.parse_args_with(|input: ParseStream| {
            while !input.is_empty() {
                let key: Ident = input.parse()?;
                if key == "validate" {
                    let content;
                    parenthesized!(content in input);
                    parse_rules(&content, &mut rules)?;
                } else {
                    return Err(syn::Error::new(key.span(), "expected `validate(...)`"));
                }
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
            }
            Ok(())
        })?;
    }
    Ok(rules)
}

fn parse_rules(input: ParseStream, out: &mut Vec<Rule>) -> syn::Result<()> {
    while !input.is_empty() {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "email" => out.push(Rule::Email),
            "required" => out.push(Rule::Required),
            "length" => {
                let content;
                parenthesized!(content in input);
                out.push(parse_length(&content)?);
            }
            "range" => {
                let content;
                parenthesized!(content in input);
                out.push(parse_range(&content)?);
            }
            "regex" => {
                let content;
                parenthesized!(content in input);
                // Inline pattern: `regex(r"^\d+$")` (a string literal — bare patterns
                // can't tokenize through `\`).
                out.push(Rule::Regex(content.parse()?));
            }
            other => {
                return Err(syn::Error::new(name.span(), format!("unknown validate rule `{other}`")));
            }
        }
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(())
}

fn parse_length(input: ParseStream) -> syn::Result<Rule> {
    let (mut min, mut max, mut equal) = (None, None, None);
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        let content;
        parenthesized!(content in input);
        let value: u64 = content.parse::<LitInt>()?.base10_parse()?;
        match key.to_string().as_str() {
            "min" => min = Some(value),
            "max" => max = Some(value),
            "equal" => equal = Some(value),
            other => return Err(syn::Error::new(key.span(), format!("unknown length arg `{other}`"))),
        }
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(Rule::Length { min, max, equal })
}

fn parse_range(input: ParseStream) -> syn::Result<Rule> {
    let (mut min, mut max) = (None, None);
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        let content;
        parenthesized!(content in input);
        let value: LitInt = content.parse()?;
        match key.to_string().as_str() {
            "min" => min = Some(value),
            "max" => max = Some(value),
            other => return Err(syn::Error::new(key.span(), format!("unknown range arg `{other}`"))),
        }
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(Rule::Range { min, max })
}

use proc_macro2::TokenStream;
use quote::quote;
use syn::{
    FnArg, GenericArgument, Ident, ItemFn, LitStr, PathArguments, ReturnType, Token, Type,
    parse::{Parse, ParseStream},
    parse2,
};

/// `#[endpoint(POST, "/live/sessions")]` or `#[endpoint(GET, "/live/sessions/{id}", "getSession")]`.
struct EndpointArgs {
    method: Ident,
    path: LitStr,
    name: Option<LitStr>,
}

impl Parse for EndpointArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let method: Ident = input.parse()?;
        input.parse::<Token![,]>()?;
        let path: LitStr = input.parse()?;
        let name = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            Some(input.parse::<LitStr>()?)
        } else {
            None
        };
        Ok(Self { method, path, name })
    }
}

pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args: EndpointArgs = match parse2(attr) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error(),
    };
    let func: ItemFn = match parse2(item) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };

    let method = args.method.to_string().to_uppercase();
    let path = args.path.value();
    let name = args.name.map(|n| n.value()).unwrap_or_else(|| auto_name(&method, &path));

    // Request body and query types come from the handler's extractors.
    let mut request_ty: Option<Type> = None;
    let mut query_ty: Option<Type> = None;
    for input in &func.sig.inputs {
        if let FnArg::Typed(arg) = input {
            if request_ty.is_none() {
                request_ty = extract_wrapped(&arg.ty, "Json");
            }
            if query_ty.is_none() {
                query_ty = extract_wrapped(&arg.ty, "Query");
            }
        }
    }

    // Response type is whatever `Json<T>` appears in the return type.
    let response_ty = match &func.sig.output {
        ReturnType::Type(_, ty) => find_json(ty),
        ReturnType::Default => None,
    };

    let request = type_ref(request_ty.as_ref());
    let query = type_ref(query_ty.as_ref());
    let response = type_ref(response_ty.as_ref());

    quote! {
        #func

        inventory::submit! {
            ::orm::registry::EndpointMeta {
                method: #method,
                path: #path,
                name: #name,
                request: #request,
                query: #query,
                response: #response,
            }
        }
    }
}

fn type_ref(ty: Option<&Type>) -> TokenStream {
    match ty {
        Some(ty) => quote! {
            ::core::option::Option::Some(::orm::registry::TypeRef {
                ts_name: || <#ty as ts_rs::TS>::name(),
                ts_output_path: || <#ty as ts_rs::TS>::output_path(),
            })
        },
        None => quote! { ::core::option::Option::None },
    }
}

/// `Json<T>` or `Valid<Json<T>>` (resp. `Query`) -> `T`.
fn extract_wrapped(ty: &Type, wrapper: &str) -> Option<Type> {
    if let Some(inner) = generic_arg(ty, wrapper) {
        return Some(inner);
    }
    let valid = generic_arg(ty, "Valid")?;
    generic_arg(&valid, wrapper)
}

/// If `ty` is `Wrapper<T, ...>`, return `T`.
fn generic_arg(ty: &Type, wrapper: &str) -> Option<Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last()?;
    if segment.ident != wrapper {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else { return None };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
}

/// Recursively search a return type for a `Json<T>` (handles `Result<…>`,
/// tuples like `(StatusCode, Json<T>)`, etc.) and return `T`.
fn find_json(ty: &Type) -> Option<Type> {
    match ty {
        Type::Path(path) => {
            let segment = path.path.segments.last()?;
            if segment.ident == "Json" {
                return generic_arg(ty, "Json");
            }
            if let PathArguments::AngleBracketed(args) = &segment.arguments {
                for arg in &args.args {
                    if let GenericArgument::Type(t) = arg {
                        if let Some(found) = find_json(t) {
                            return Some(found);
                        }
                    }
                }
            }
            None
        }
        Type::Tuple(tuple) => tuple.elems.iter().find_map(find_json),
        _ => None,
    }
}

/// `POST /live/sessions/{id}/end` -> `postLiveSessionsByIdEnd` (fallback when no
/// explicit name is given).
fn auto_name(method: &str, path: &str) -> String {
    let mut name = method.to_lowercase();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(param) => {
                name.push_str("By");
                name.push_str(&pascal(param));
            }
            None => name.push_str(&pascal(segment)),
        }
    }
    name
}

fn pascal(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

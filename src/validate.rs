//! orm's owned validation stack.
//!
//! - [`Validate`] — orm's validation trait, implemented for request types by the
//!   `#[api(validate(...))]` codegen, so consumers depend on `orm`, not `validator`.
//! - Re-exported `validator` runtime checks ([`ValidateEmail`], [`ValidateLength`],
//!   …) — the generated `impl Validate` bodies call these; orm pins `validator`
//!   internally so the actual checks aren't reimplemented.
//! - [`Valid`] — an axum extractor that runs the inner extractor then validates,
//!   returning `400 { "error": "<message>" }` on failure.

use std::ops::Deref;

use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};

pub use regex::Regex;
pub use validator::{
    AsRegex, ValidateEmail, ValidateLength, ValidateRange, ValidateRegex, ValidateRequired,
    ValidationError, ValidationErrors,
};

/// orm's validation trait. The `#[api(validate(...))]` macro generates the impl;
/// types with no rules get an empty (always-`Ok`) impl so they work with [`Valid`].
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationErrors>;
}

/// Extractor that runs `E` (e.g. `Json<T>` or `Query<T>`) then validates the value.
/// `Valid(Json(payload)): Valid<Json<T>>` destructures to `T`; a bare
/// `payload: Valid<Json<T>>` reaches `T`'s fields through `Deref` (`Valid` → `E` → `T`).
pub struct Valid<E>(pub E);

impl<E> Deref for Valid<E> {
    type Target = E;
    fn deref(&self) -> &E {
        &self.0
    }
}

// Two impls so `Valid` composes like any extractor: `FromRequest` for body
// extractors (`Json`), `FromRequestParts` for parts extractors (`Query`) — the
// latter lets several `Valid<Query<_>>` coexist in one handler. They don't
// collide: the `FromRequest` impl requires `E: FromRequest` (the direct marker,
// which `Query` lacks), while `Query`'s `FromRequest` comes via axum's parts
// blanket (a different marker).
impl<S, E> FromRequest<S> for Valid<E>
where
    S: Send + Sync,
    E: FromRequest<S>,
    E::Rejection: IntoResponse,
    E: Deref,
    <E as Deref>::Target: Validate,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let inner = E::from_request(req, state).await.map_err(IntoResponse::into_response)?;
        validated(inner)
    }
}

impl<S, E> FromRequestParts<S> for Valid<E>
where
    S: Send + Sync,
    E: FromRequestParts<S>,
    E::Rejection: IntoResponse,
    E: Deref,
    <E as Deref>::Target: Validate,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let inner = E::from_request_parts(parts, state).await.map_err(IntoResponse::into_response)?;
        validated(inner)
    }
}

fn validated<E>(inner: E) -> Result<Valid<E>, Response>
where
    E: Deref,
    <E as Deref>::Target: Validate,
{
    match inner.validate() {
        Ok(()) => Ok(Valid(inner)),
        Err(errors) => Err(rejection(&errors)),
    }
}

/// `400 { "error": "<summary>", "fields": { "<field>": "<message>" } }` — the
/// `fields` map lets the client surface each error on its form field (the
/// generated client parses it into `ApiError.fields`).
fn rejection(errors: &ValidationErrors) -> Response {
    let mut fields = serde_json::Map::new();
    for (field, field_errors) in errors.field_errors() {
        if let Some(first) = field_errors.first() {
            let message = first
                .message
                .clone()
                .map(|m| m.into_owned())
                .unwrap_or_else(|| first.code.clone().into_owned());
            fields.insert(field.to_string(), serde_json::Value::String(message));
        }
    }

    let summary = if fields.is_empty() {
        "validation failed".to_string()
    } else {
        format!("invalid {}", fields.keys().cloned().collect::<Vec<_>>().join(", "))
    };

    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({ "error": summary, "fields": fields })),
    )
        .into_response()
}

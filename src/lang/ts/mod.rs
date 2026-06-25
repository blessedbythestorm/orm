//! TypeScript code generation: the type [`backend`], the [`client`] generator,
//! and the [`validator`] schema backends (valibot, ...).

mod backend;
mod client;
mod runtime;
mod validator;

pub use backend::TypeScript;
pub use client::{generate_client, generate_result};
pub use validator::Valibot;

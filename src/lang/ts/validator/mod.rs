//! TypeScript validator backends. [`Valibot`] is the default; add e.g. `zod` as
//! a sibling module.

mod valibot;

pub use valibot::Valibot;

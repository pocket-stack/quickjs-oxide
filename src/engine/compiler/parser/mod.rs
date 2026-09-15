//! Source parsing and construction-only function state.

pub(super) mod builder;
mod calls;
pub(super) mod context;
mod control;
mod declarations;
pub(super) mod diagnostics;
pub(super) mod entry;
mod expressions;
pub(super) mod literals;
mod loops;
mod parameters;
mod scopes;
mod statements;
pub(super) mod tokens;

//! Complete interpreter, organized by ownership and execution responsibility.
pub mod api;
pub(crate) mod atom;
pub(crate) mod builtins;
pub(crate) mod code;
pub(crate) mod compiler;
pub(crate) mod heap;
pub(crate) mod host;
pub(crate) mod jobs;
pub(crate) mod modules;
pub(crate) mod object;
pub(crate) mod realm;
pub(crate) mod value;
pub(crate) mod vm;

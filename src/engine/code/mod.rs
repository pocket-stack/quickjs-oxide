//! Instructions, compilation drafts, verification and runtime-rooted code.
pub mod bytecode;
pub(crate) mod bytecode_validation;
pub mod debug;
pub mod function;
pub mod module;
pub mod rooted;

mod binary_object;

mod binary_object_publish;

pub(crate) mod bytecode_publish;

pub(crate) mod dynamic_import_policy;

pub(crate) mod runtime;

pub(crate) mod dynamic_source;

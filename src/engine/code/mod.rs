//! Instructions, compilation drafts, verification and runtime-rooted code.
pub mod bytecode;
pub(crate) mod bytecode_validation;
pub mod debug;
pub mod function;
pub(crate) mod instruction;
pub mod module;
pub mod rooted;

mod binary_object;

mod binary_object_publish;

pub(crate) mod bytecode_publish;
pub(crate) mod verify;

pub(crate) mod dynamic_import_policy;

pub(crate) mod runtime;

pub(crate) mod dynamic_source;

mod executable;

#[cfg(feature = "stack-vm")]
pub(crate) mod fusion;

//! A pure-Rust rewrite of `QuickJS` aiming at semantic feature parity with the
//! pinned upstream release.
//!
//! The implementation deliberately follows `QuickJS`'s major runtime boundaries:
//! source is compiled to stack bytecode, bytecode executes inside a context,
//! and contexts share a runtime-owned heap and atom table.

pub mod atom;
pub mod bigint;
pub mod bytecode;
pub mod compiler;
pub mod debug;
pub mod error;
pub mod function;
pub mod heap;
pub mod lexer;
pub mod number;
pub mod object;
pub mod property;
pub mod runtime;
pub mod shape;
pub mod value;
pub mod vm;

pub use bigint::{BigIntError, JsBigInt};
pub use compiler::CompileOptions;
pub use debug::{
    DebugInfoMode, LineColumn, Pc2LineEntry, Pc2LineTable, QuickJsSourceLocator, SourceOffset,
};
pub use error::{Error, ErrorKind, SourceLocation, SourceSpan};
pub use function::FunctionBytecodeRef;
pub use object::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, SymbolRef, WellKnownSymbol,
};
pub use runtime::{Context, EvalOptions, Runtime, RuntimeError};
pub use value::{JsString, Value};

/// The exact upstream release whose observable behavior is the compatibility
/// baseline for this crate.
pub const QUICKJS_COMPAT_VERSION: &str = "2026-06-04";

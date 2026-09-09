//! A pure-Rust rewrite of `QuickJS` aiming at semantic feature parity with the
//! pinned upstream release.
//!
//! The implementation deliberately follows `QuickJS`'s major runtime boundaries:
//! source is compiled to stack bytecode, bytecode executes inside a context,
//! and contexts share a runtime-owned heap and atom table.

/// Interpreter implementation; embedders use [`engine::api`].
///
/// Old root API aliases are deliberately unavailable:
/// ```compile_fail
/// use quickjs_oxide::Runtime;
/// ```
/// Internal engine modules are not embedding interfaces:
/// ```compile_fail
/// use quickjs_oxide::engine::heap::Heap;
/// ```
/// ```compile_fail
/// use quickjs_oxide::engine::vm::Vm;
/// ```
pub mod engine;
pub mod regexp;
pub mod source;

/// The version of this quickjs-oxide engine crate.
pub const QUICKJS_OXIDE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The exact upstream release whose observable behavior is the compatibility
/// baseline for this crate.
pub const QUICKJS_COMPAT_VERSION: &str = "2026-06-04";

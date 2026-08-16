//! Release-pinned QuickJS binary-object support.
//!
//! The first implementation slice is deliberately limited to the pure wire
//! layer. Runtime values, heap identities, and public `Context` APIs remain
//! outside this module until their observable semantics can be admitted as a
//! separate milestone.

// The wire layer is intentionally staged before its runtime consumer. Keep the
// allowance local so the rest of `runtime` still receives dead-code warnings.
#[allow(dead_code)]
pub(in crate::runtime) mod atoms;

#[allow(dead_code)]
pub(in crate::runtime) mod code;

#[allow(dead_code)]
pub(in crate::runtime) mod graph;

#[allow(dead_code)]
pub(in crate::runtime) mod pinned_atoms;

#[allow(dead_code)]
pub(in crate::runtime) mod pinned_opcodes;

#[allow(dead_code)]
pub(in crate::runtime) mod wire;

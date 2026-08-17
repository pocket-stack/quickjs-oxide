//! Release-pinned QuickJS binary-object support.
//!
//! The first implementation slice is deliberately limited to the pure wire
//! layer. Runtime values, heap identities, and public `Context` APIs remain
//! outside this module until their observable semantics can be admitted as a
//! separate milestone.

// The wire layer is intentionally staged before its runtime consumer. Keep the
// allowance local so the rest of `runtime` still receives dead-code warnings.
#[allow(dead_code)]
mod atoms;

#[allow(dead_code)]
mod code;

#[allow(dead_code)]
mod function_envelope;

#[allow(dead_code)]
mod bytecode_image;

#[allow(dead_code)]
mod graph;

#[allow(dead_code)]
mod pinned_atoms;

#[allow(dead_code)]
mod pinned_opcodes;

#[allow(dead_code)]
mod read_cursor;

#[allow(dead_code)]
mod wire;

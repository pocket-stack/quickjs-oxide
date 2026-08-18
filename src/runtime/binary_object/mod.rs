//! Release-pinned QuickJS binary-object support.
//!
//! The archive reader remains heap-independent. Its runtime-facing products
//! are narrow, non-executable drafts which a separate publication bridge must
//! translate and verify before entering the heap.

// The wire layer is intentionally staged before its runtime consumer. Keep the
// allowance local so the rest of `runtime` still receives dead-code warnings.
#[allow(dead_code)]
mod atoms;

#[allow(dead_code)]
mod code;

#[allow(dead_code)]
mod function_envelope;

#[allow(dead_code)]
mod function_translate;

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
mod ordinary_leaf;

#[allow(dead_code)]
mod scalar_script;

#[allow(dead_code)]
mod wire;

#[allow(unused_imports)]
pub(super) use scalar_script::{
    ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft,
    decode_trusted_scalar_script,
};

#[allow(unused_imports)]
pub(super) use ordinary_leaf::{
    DetachedPrimitive, OrdinaryLeafBinaryOp, OrdinaryLeafDraft, OrdinaryLeafMetadataDraft,
    OrdinaryLeafOp, OrdinaryLeafPredicateOp, OrdinaryLeafReadError, OrdinaryLeafStackOp,
    OrdinaryLeafUnaryOp, RootFunctionConstantSelector, decode_trusted_ordinary_leaf,
};

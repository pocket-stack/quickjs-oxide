//! Read-only caller authority supplied by an eval publication request.

use super::metadata::{EvalCallerProfile, EvalKind, EvalRootBinding};
use crate::engine::value::JsString;

/// Borrows the invocation's exact binding order and profile without carrying
/// parser options or copying the caller's binding arrays into verification.
pub(crate) struct EvalPublicationInput<'a> {
    pub kind: EvalKind,
    pub caller_strict: bool,
    pub bindings: &'a [EvalRootBinding<JsString>],
    pub caller_profile: &'a EvalCallerProfile,
    pub super_call_allowed: bool,
    pub super_allowed: bool,
    pub arguments_forbidden: bool,
}

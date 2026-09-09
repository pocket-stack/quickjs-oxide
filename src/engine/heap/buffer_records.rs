use super::*;

/// QuickJS `JSArrayBuffer` storage owned directly by one branded object.
///
/// `max_byte_length == None` denotes a fixed-length buffer. A detached buffer
/// keeps its resizable/fixed identity and maximum while releasing all bytes;
/// its observable byte length is therefore zero without conflating an
/// attached empty buffer with a detached one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArrayBufferData {
    pub bytes: Vec<u8>,
    pub max_byte_length: Option<u32>,
    pub detached: bool,
}

/// QuickJS `JS_CLASS_SHARED_ARRAY_BUFFER` wrapper-local state.
///
/// The handle owns no arena edge: it keeps the shared backing alive through
/// `Arc`, while byte length and maximum length remain local to this wrapper.
/// Cloning the handle therefore mirrors QuickJS's SAB structured-clone hook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedArrayBufferData {
    pub handle: SharedBufferHandle,
}

/// Borrow-free snapshot of one ArrayBuffer backing-store state.
///
/// Runtime DataView operations use this value to finish observable validation
/// without retaining a heap borrow across coercions or subsequent mutations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArrayBufferState {
    pub byte_length: u32,
    pub max_byte_length: Option<u32>,
    pub detached: bool,
}

/// Shared ArrayBuffer-view layout carried by a genuine DataView.
///
/// `fixed_byte_length == None` denotes a length-tracking view over a resizable
/// ArrayBuffer. Detach and resize may make this structurally valid view
/// temporarily out of bounds; that observable state is checked at access time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayBufferViewData {
    pub buffer: ObjectId,
    pub byte_offset: u32,
    pub fixed_byte_length: Option<u32>,
}

/// Shared integer-indexed view layout carried by all twelve TypedArray classes.
///
/// The nested byte-oriented view is deliberately the same substrate used by
/// DataView and mirrors QuickJS's `JSTypedArray.length`. `None` denotes a
/// length-tracking view over a resizable ArrayBuffer; fixed byte lengths are
/// validated to be divisible by the selected element width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypedArrayData {
    pub view: ArrayBufferViewData,
    pub element: TypedArrayElementKind,
}

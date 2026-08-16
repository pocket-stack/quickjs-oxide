//! Non-executable semantic model for one complete BC5 bytecode image.
//!
//! Every atom-bearing field in this module has already been relocated out of
//! the raw header-slot namespace. The model deliberately has no verifier,
//! materializer, runtime-heap handle, or execution entry point.

use super::super::function_envelope::{
    ClosureVariableFlags, FunctionFlags, JsMode, LocalVariableFlags, ScopeLink,
};
use super::super::graph::decode::MachineSource;
use super::super::graph::model::{NodeId, WireNodeCarrier, WireValue};
use super::super::pinned_opcodes::PinnedOpcode;
use super::super::wire::WireString;
use super::decode::AuthenticatedFunction;
use super::{ImageAtom, ImageKey};

/// Zero-based identity of one [`FunctionRecord`] in a complete image.
///
/// Construction consumes a decoder-private [`AuthenticatedFunction`] proof
/// produced only after its same-source function slot is complete. Sibling
/// binary-object layers therefore cannot forge an identity and ask a data
/// machine to authenticate it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct FunctionId {
    source: MachineSource,
    index: u32,
}

impl FunctionId {
    pub(in crate::runtime::binary_object) const fn source(self) -> MachineSource {
        self.source
    }

    #[must_use]
    pub(in crate::runtime) const fn zero_based(self) -> u32 {
        self.index
    }

    #[must_use]
    pub(in crate::runtime) const fn as_usize(self) -> usize {
        self.index as usize
    }
}

/// A data value or an opaque function identity in one whole-image traversal.
///
/// The representation is intentionally private. In particular, callers cannot
/// pattern-match a function into a raw integer or reconstruct a data value
/// around a foreign [`NodeId`]. The shared data machine receives only the
/// narrow classifier/conversion methods below.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageValue {
    kind: ImageValueKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ImageValueKind {
    Data(WireValue),
    Function(FunctionId),
}

impl ImageValue {
    pub(in crate::runtime::binary_object) fn from_wire(value: WireValue) -> Self {
        Self {
            kind: ImageValueKind::Data(value),
        }
    }

    pub(in crate::runtime::binary_object) const fn as_wire(
        &self,
    ) -> Result<&WireValue, FunctionId> {
        match &self.kind {
            ImageValueKind::Data(value) => Ok(value),
            ImageValueKind::Function(function) => Err(*function),
        }
    }

    pub(in crate::runtime::binary_object) fn into_wire(self) -> Result<WireValue, FunctionId> {
        match self.kind {
            ImageValueKind::Data(value) => Ok(value),
            ImageValueKind::Function(function) => Err(function),
        }
    }

    pub(super) fn from_function(function: AuthenticatedFunction) -> Self {
        Self {
            kind: ImageValueKind::Function(FunctionId {
                source: function.source(),
                index: function.index(),
            }),
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn function_id(&self) -> Option<FunctionId> {
        match self.kind {
            ImageValueKind::Data(_) => None,
            ImageValueKind::Function(function) => Some(function),
        }
    }
}

/// One object identity in the shared whole-image arena.
pub(super) type ImageNode = WireNodeCarrier<ImageValue, ImageKey>;

/// One relocated instruction boundary in an owned native-code payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageInstructionSpan {
    offset: u32,
    opcode: PinnedOpcode,
}

impl ImageInstructionSpan {
    pub(super) const fn new(offset: u32, opcode: PinnedOpcode) -> Self {
        Self { offset, opcode }
    }

    #[must_use]
    pub(in crate::runtime) const fn offset(self) -> u32 {
        self.offset
    }

    #[must_use]
    pub(in crate::runtime) const fn opcode(self) -> PinnedOpcode {
        self.opcode
    }
}

/// One native-code atom operand relocated into the image-wide atom namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageRelocation {
    operand_offset: u32,
    atom: ImageAtom,
}

impl ImageRelocation {
    pub(super) const fn new(operand_offset: u32, atom: ImageAtom) -> Self {
        Self {
            operand_offset,
            atom,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn operand_offset(self) -> u32 {
        self.operand_offset
    }

    #[must_use]
    pub(super) const fn atom(self) -> ImageAtom {
        self.atom
    }
}

/// Owned native bytes and their relocated structural sidecars.
///
/// The bytes remain a non-executable archival payload. Semantic atom identity
/// lives in `atom_relocations`; no method here rebuilds native runtime atoms or
/// publishes executable bytecode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageCode {
    bytes: Box<[u8]>,
    instructions: Box<[ImageInstructionSpan]>,
    atom_relocations: Box<[ImageRelocation]>,
}

impl ImageCode {
    pub(super) const fn new(
        bytes: Box<[u8]>,
        instructions: Box<[ImageInstructionSpan]>,
        atom_relocations: Box<[ImageRelocation]>,
    ) -> Self {
        Self {
            bytes,
            instructions,
            atom_relocations,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub(in crate::runtime) const fn instructions(&self) -> &[ImageInstructionSpan] {
        &self.instructions
    }

    #[must_use]
    pub(in crate::runtime) const fn atom_relocations(&self) -> &[ImageRelocation] {
        &self.atom_relocations
    }
}

/// One relocated local-variable descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageLocalVariable {
    name: ImageAtom,
    scope_next: ScopeLink,
    variable_reference_index: u16,
    flags: LocalVariableFlags,
}

impl ImageLocalVariable {
    pub(super) const fn new(
        name: ImageAtom,
        scope_next: ScopeLink,
        variable_reference_index: u16,
        flags: LocalVariableFlags,
    ) -> Self {
        Self {
            name,
            scope_next,
            variable_reference_index,
            flags,
        }
    }

    #[must_use]
    pub(super) const fn name(&self) -> ImageAtom {
        self.name
    }

    #[must_use]
    pub(in crate::runtime) const fn scope_next(&self) -> ScopeLink {
        self.scope_next
    }

    #[must_use]
    pub(in crate::runtime) const fn variable_reference_index(&self) -> u16 {
        self.variable_reference_index
    }

    #[must_use]
    pub(in crate::runtime) const fn flags(&self) -> LocalVariableFlags {
        self.flags
    }
}

/// One relocated closure-variable descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageClosureVariable {
    name: ImageAtom,
    variable_index: u16,
    flags: ClosureVariableFlags,
}

impl ImageClosureVariable {
    pub(super) const fn new(
        name: ImageAtom,
        variable_index: u16,
        flags: ClosureVariableFlags,
    ) -> Self {
        Self {
            name,
            variable_index,
            flags,
        }
    }

    #[must_use]
    pub(super) const fn name(&self) -> ImageAtom {
        self.name
    }

    #[must_use]
    pub(in crate::runtime) const fn variable_index(&self) -> u16 {
        self.variable_index
    }

    #[must_use]
    pub(in crate::runtime) const fn flags(&self) -> ClosureVariableFlags {
        self.flags
    }
}

/// Relocated optional debug payload for one function record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageFunctionDebug {
    filename: ImageAtom,
    pc2line: Box<[u8]>,
    source: Box<[u8]>,
}

impl ImageFunctionDebug {
    pub(super) const fn new(filename: ImageAtom, pc2line: Box<[u8]>, source: Box<[u8]>) -> Self {
        Self {
            filename,
            pc2line,
            source,
        }
    }

    #[must_use]
    pub(super) const fn filename(&self) -> ImageAtom {
        self.filename
    }

    #[must_use]
    pub(in crate::runtime) const fn pc2line(&self) -> &[u8] {
        &self.pc2line
    }

    #[must_use]
    pub(in crate::runtime) const fn source(&self) -> &[u8] {
        &self.source
    }
}

/// Complete atom-relocated metadata for one FunctionBytecode occurrence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageFunctionEnvelope {
    flags: FunctionFlags,
    js_mode: JsMode,
    name: ImageAtom,
    argument_count: u16,
    variable_count: u16,
    defined_argument_count: u16,
    stack_size: u16,
    variable_reference_count: u16,
    locals: Box<[ImageLocalVariable]>,
    closures: Box<[ImageClosureVariable]>,
    code: ImageCode,
    debug: Option<ImageFunctionDebug>,
}

impl ImageFunctionEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        flags: FunctionFlags,
        js_mode: JsMode,
        name: ImageAtom,
        argument_count: u16,
        variable_count: u16,
        defined_argument_count: u16,
        stack_size: u16,
        variable_reference_count: u16,
        locals: Box<[ImageLocalVariable]>,
        closures: Box<[ImageClosureVariable]>,
        code: ImageCode,
        debug: Option<ImageFunctionDebug>,
    ) -> Self {
        Self {
            flags,
            js_mode,
            name,
            argument_count,
            variable_count,
            defined_argument_count,
            stack_size,
            variable_reference_count,
            locals,
            closures,
            code,
            debug,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn flags(&self) -> FunctionFlags {
        self.flags
    }

    #[must_use]
    pub(in crate::runtime) const fn js_mode(&self) -> JsMode {
        self.js_mode
    }

    #[must_use]
    pub(super) const fn name(&self) -> ImageAtom {
        self.name
    }

    #[must_use]
    pub(in crate::runtime) const fn argument_count(&self) -> u16 {
        self.argument_count
    }

    #[must_use]
    pub(in crate::runtime) const fn variable_count(&self) -> u16 {
        self.variable_count
    }

    #[must_use]
    pub(in crate::runtime) const fn defined_argument_count(&self) -> u16 {
        self.defined_argument_count
    }

    #[must_use]
    pub(in crate::runtime) const fn stack_size(&self) -> u16 {
        self.stack_size
    }

    #[must_use]
    pub(in crate::runtime) const fn variable_reference_count(&self) -> u16 {
        self.variable_reference_count
    }

    #[must_use]
    pub(in crate::runtime) const fn locals(&self) -> &[ImageLocalVariable] {
        &self.locals
    }

    #[must_use]
    pub(in crate::runtime) const fn closures(&self) -> &[ImageClosureVariable] {
        &self.closures
    }

    #[must_use]
    pub(in crate::runtime) const fn code(&self) -> &ImageCode {
        &self.code
    }

    #[must_use]
    pub(in crate::runtime) const fn debug(&self) -> Option<&ImageFunctionDebug> {
        self.debug.as_ref()
    }
}

/// One complete function record with its constant pool in wire order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionRecord {
    envelope: ImageFunctionEnvelope,
    constants: Box<[ImageValue]>,
}

impl FunctionRecord {
    pub(super) const fn new(envelope: ImageFunctionEnvelope, constants: Box<[ImageValue]>) -> Self {
        Self {
            envelope,
            constants,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn envelope(&self) -> &ImageFunctionEnvelope {
        &self.envelope
    }

    #[must_use]
    pub(in crate::runtime) const fn constants(&self) -> &[ImageValue] {
        &self.constants
    }
}

/// Complete, heap-independent, and deliberately non-executable BC5 image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct BytecodeImage {
    source: MachineSource,
    atoms: Box<[WireString]>,
    nodes: Box<[ImageNode]>,
    ref_table: Box<[NodeId]>,
    functions: Box<[FunctionRecord]>,
    root: ImageValue,
}

impl BytecodeImage {
    pub(super) const fn new(
        source: MachineSource,
        atoms: Box<[WireString]>,
        nodes: Box<[ImageNode]>,
        ref_table: Box<[NodeId]>,
        functions: Box<[FunctionRecord]>,
        root: ImageValue,
    ) -> Self {
        Self {
            source,
            atoms,
            nodes,
            ref_table,
            functions,
            root,
        }
    }

    /// Return the image-local dynamic strings indexed by `ImageAtom::Dynamic`.
    #[must_use]
    pub(in crate::runtime) const fn atoms(&self) -> &[WireString] {
        &self.atoms
    }

    #[must_use]
    pub(super) const fn nodes(&self) -> &[ImageNode] {
        &self.nodes
    }

    #[must_use]
    pub(in crate::runtime) const fn reference_table(&self) -> &[NodeId] {
        &self.ref_table
    }

    #[must_use]
    pub(in crate::runtime) const fn functions(&self) -> &[FunctionRecord] {
        &self.functions
    }

    #[must_use]
    pub(in crate::runtime) fn function(&self, id: FunctionId) -> Option<&FunctionRecord> {
        (id.source() == self.source)
            .then(|| self.functions.get(id.as_usize()))
            .flatten()
    }

    #[must_use]
    pub(in crate::runtime) const fn root(&self) -> &ImageValue {
        &self.root
    }
}

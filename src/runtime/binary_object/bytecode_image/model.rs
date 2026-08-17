//! Non-executable semantic model for one complete BC5 bytecode image.
//!
//! Every atom-bearing field in this module has already been relocated out of
//! the raw header-slot namespace. The model deliberately has no verifier,
//! materializer, runtime-heap handle, or execution entry point.

use std::num::NonZeroU8;

use super::super::function_envelope::{
    ClosureVariableFlags, FunctionFlags, JsMode, LocalVariableFlags, ScopeLink,
};
use super::super::graph::decode::MachineSource;
use super::super::graph::model::{NodeId, WireNodeCarrier, WireValue};
use super::super::graph::sab_transport::SabArchiveOccurrence;
use super::super::pinned_opcodes::PinnedOpcode;
use super::super::wire::WireString;
use super::decode::{AuthenticatedFunction, AuthenticatedModule};
use super::{ImageAtom, ImageKey};

/// QuickJS 2026-06-04's release-pinned atom identity for `<eval>`.
const PINNED_EVAL_ATOM_RAW: u32 = 84;

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

/// Zero-based identity of one [`ModuleRecord`] in a complete image.
///
/// Construction consumes a decoder-private [`AuthenticatedModule`] proof
/// produced only after its same-source module slot is complete. As with
/// [`FunctionId`], the raw index cannot be forged by sibling binary-object
/// layers.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct ModuleId {
    source: MachineSource,
    index: u32,
}

impl ModuleId {
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

/// Opaque non-data identity admitted by the shared graph traversal.
///
/// Both variants retain the same unforgeable [`MachineSource`] issued to the
/// traversal that authenticated their table slot.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime::binary_object) enum ImageOpaque {
    Function(FunctionId),
    Module(ModuleId),
}

impl ImageOpaque {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn source(self) -> MachineSource {
        match self {
            Self::Function(function) => function.source(),
            Self::Module(module) => module.source(),
        }
    }
}

/// A data value, function identity, or module identity in one whole-image
/// traversal.
///
/// The representation is intentionally private. In particular, callers cannot
/// reconstruct an opaque identity from a raw integer or reconstruct a data
/// value around a foreign [`NodeId`]. The shared data machine receives only
/// the narrow classifier/conversion methods below.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ImageValue {
    kind: ImageValueKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ImageValueKind {
    Data(WireValue),
    Function(FunctionId),
    Module(ModuleId),
}

impl ImageValue {
    pub(in crate::runtime::binary_object) fn from_wire(value: WireValue) -> Self {
        Self {
            kind: ImageValueKind::Data(value),
        }
    }

    pub(in crate::runtime::binary_object) const fn as_wire(
        &self,
    ) -> Result<&WireValue, ImageOpaque> {
        match &self.kind {
            ImageValueKind::Data(value) => Ok(value),
            ImageValueKind::Function(function) => Err(ImageOpaque::Function(*function)),
            ImageValueKind::Module(module) => Err(ImageOpaque::Module(*module)),
        }
    }

    pub(in crate::runtime::binary_object) fn into_wire(self) -> Result<WireValue, ImageOpaque> {
        match self.kind {
            ImageValueKind::Data(value) => Ok(value),
            ImageValueKind::Function(function) => Err(ImageOpaque::Function(function)),
            ImageValueKind::Module(module) => Err(ImageOpaque::Module(module)),
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

    pub(super) fn from_module(module: AuthenticatedModule) -> Self {
        Self {
            kind: ImageValueKind::Module(ModuleId {
                source: module.source(),
                index: module.index(),
            }),
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn function_id(&self) -> Option<FunctionId> {
        match self.kind {
            ImageValueKind::Function(function) => Some(function),
            ImageValueKind::Data(_) | ImageValueKind::Module(_) => None,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn module_id(&self) -> Option<ModuleId> {
        match self.kind {
            ImageValueKind::Module(module) => Some(module),
            ImageValueKind::Data(_) | ImageValueKind::Function(_) => None,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn opaque(&self) -> Option<ImageOpaque> {
        match self.kind {
            ImageValueKind::Data(_) => None,
            ImageValueKind::Function(function) => Some(ImageOpaque::Function(function)),
            ImageValueKind::Module(module) => Some(ImageOpaque::Module(module)),
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

    /// Whether this local carries QuickJS's atom-zero sentinel as its name.
    ///
    /// Scalar admission needs only this fact, not the image-private atom
    /// representation or a way to reconstruct it.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn name_is_null(&self) -> bool {
        matches!(self.name, ImageAtom::Null)
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

    /// Whether this record names the pinned QuickJS `<eval>` atom.
    ///
    /// Keep the raw atom identity sealed inside the image model: scalar
    /// admission receives only the exact-shape predicate it requires.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn name_is_pinned_eval(&self) -> bool {
        match self.name {
            ImageAtom::Predefined(atom) => atom.raw() == PINNED_EVAL_ATOM_RAW,
            ImageAtom::Null | ImageAtom::Index(_) | ImageAtom::Dynamic(_) => false,
        }
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

/// One module request in wire order.
///
/// QuickJS stores the request attributes through the generic object codec, so
/// the heap-independent image deliberately retains an arbitrary [`ImageValue`]
/// instead of narrowing it to an ordinary object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ModuleRequest {
    name: ImageAtom,
    attributes: ImageValue,
}

impl ModuleRequest {
    pub(super) const fn new(name: ImageAtom, attributes: ImageValue) -> Self {
        Self { name, attributes }
    }

    #[must_use]
    pub(super) const fn name(&self) -> ImageAtom {
        self.name
    }

    #[must_use]
    pub(in crate::runtime) const fn attributes(&self) -> &ImageValue {
        &self.attributes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ModuleExportBinding {
    Local {
        variable_index: u32,
    },
    NonLocal {
        export_type: NonZeroU8,
        request_index: u32,
        local_name: ImageAtom,
    },
}

/// One module export in wire order.
///
/// Wire type zero names a local-variable index. Every non-zero wire type is
/// preserved exactly rather than being collapsed into the currently known
/// indirect-export category.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ModuleExport {
    binding: ModuleExportBinding,
    export_name: ImageAtom,
}

impl ModuleExport {
    pub(super) const fn new_local(variable_index: u32, export_name: ImageAtom) -> Self {
        Self {
            binding: ModuleExportBinding::Local { variable_index },
            export_name,
        }
    }

    pub(super) const fn new_non_local(
        export_type: NonZeroU8,
        request_index: u32,
        local_name: ImageAtom,
        export_name: ImageAtom,
    ) -> Self {
        Self {
            binding: ModuleExportBinding::NonLocal {
                export_type,
                request_index,
                local_name,
            },
            export_name,
        }
    }

    /// Return the exact BC5 export type. Zero denotes a local export.
    #[must_use]
    pub(in crate::runtime) const fn export_type(&self) -> u8 {
        match self.binding {
            ModuleExportBinding::Local { .. } => 0,
            ModuleExportBinding::NonLocal { export_type, .. } => export_type.get(),
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn local_variable_index(&self) -> Option<u32> {
        match self.binding {
            ModuleExportBinding::Local { variable_index } => Some(variable_index),
            ModuleExportBinding::NonLocal { .. } => None,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn request_index(&self) -> Option<u32> {
        match self.binding {
            ModuleExportBinding::Local { .. } => None,
            ModuleExportBinding::NonLocal { request_index, .. } => Some(request_index),
        }
    }

    #[must_use]
    pub(super) const fn local_name(&self) -> Option<ImageAtom> {
        match self.binding {
            ModuleExportBinding::Local { .. } => None,
            ModuleExportBinding::NonLocal { local_name, .. } => Some(local_name),
        }
    }

    #[must_use]
    pub(super) const fn export_name(&self) -> ImageAtom {
        self.export_name
    }
}

/// One module import in wire order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ModuleImport {
    variable_index: u32,
    is_star: bool,
    import_name: ImageAtom,
    request_index: u32,
}

impl ModuleImport {
    pub(super) const fn new(
        variable_index: u32,
        is_star: bool,
        import_name: ImageAtom,
        request_index: u32,
    ) -> Self {
        Self {
            variable_index,
            is_star,
            import_name,
            request_index,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn variable_index(&self) -> u32 {
        self.variable_index
    }

    #[must_use]
    pub(in crate::runtime) const fn is_star(&self) -> bool {
        self.is_star
    }

    #[must_use]
    pub(super) const fn import_name(&self) -> ImageAtom {
        self.import_name
    }

    #[must_use]
    pub(in crate::runtime) const fn request_index(&self) -> u32 {
        self.request_index
    }
}

/// One complete QuickJS Module record with its nested values retained by
/// identity in the same whole-image traversal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ModuleRecord {
    name: ImageAtom,
    requests: Box<[ModuleRequest]>,
    exports: Box<[ModuleExport]>,
    star_export_request_indices: Box<[u32]>,
    imports: Box<[ModuleImport]>,
    has_tla: bool,
    func_obj: ImageValue,
}

impl ModuleRecord {
    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new(
        name: ImageAtom,
        requests: Box<[ModuleRequest]>,
        exports: Box<[ModuleExport]>,
        star_export_request_indices: Box<[u32]>,
        imports: Box<[ModuleImport]>,
        has_tla: bool,
        func_obj: ImageValue,
    ) -> Self {
        Self {
            name,
            requests,
            exports,
            star_export_request_indices,
            imports,
            has_tla,
            func_obj,
        }
    }

    #[must_use]
    pub(super) const fn name(&self) -> ImageAtom {
        self.name
    }

    #[must_use]
    pub(in crate::runtime) const fn requests(&self) -> &[ModuleRequest] {
        &self.requests
    }

    #[must_use]
    pub(in crate::runtime) const fn exports(&self) -> &[ModuleExport] {
        &self.exports
    }

    #[must_use]
    pub(in crate::runtime) const fn star_export_request_indices(&self) -> &[u32] {
        &self.star_export_request_indices
    }

    #[must_use]
    pub(in crate::runtime) const fn imports(&self) -> &[ModuleImport] {
        &self.imports
    }

    #[must_use]
    pub(in crate::runtime) const fn has_tla(&self) -> bool {
        self.has_tla
    }

    #[must_use]
    pub(in crate::runtime) const fn func_obj(&self) -> &ImageValue {
        &self.func_obj
    }
}

/// Completed atom-table evidence retained by a whole decoded image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ImageAtomSummary {
    input_slot_count: u32,
    dynamic: Box<[WireString]>,
}

impl ImageAtomSummary {
    pub(super) const fn new(input_slot_count: u32, dynamic: Box<[WireString]>) -> Self {
        Self {
            input_slot_count,
            dynamic,
        }
    }
}

/// Complete, heap-independent, and deliberately non-executable BC5 image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct BytecodeImage {
    source: MachineSource,
    atoms: ImageAtomSummary,
    nodes: Box<[ImageNode]>,
    ref_table: Box<[NodeId]>,
    functions: Box<[FunctionRecord]>,
    modules: Box<[ModuleRecord]>,
    root: ImageValue,
}

impl BytecodeImage {
    pub(super) const fn new(
        source: MachineSource,
        atoms: ImageAtomSummary,
        nodes: Box<[ImageNode]>,
        ref_table: Box<[NodeId]>,
        functions: Box<[FunctionRecord]>,
        modules: Box<[ModuleRecord]>,
        root: ImageValue,
    ) -> Self {
        Self {
            source,
            atoms,
            nodes,
            ref_table,
            functions,
            modules,
            root,
        }
    }

    /// Return the number of raw atom slots declared by the input BC5 header.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn input_atom_slot_count(&self) -> u32 {
        self.atoms.input_slot_count
    }

    /// Return the image-local dynamic strings indexed by `ImageAtom::Dynamic`.
    #[must_use]
    pub(in crate::runtime) const fn atoms(&self) -> &[WireString] {
        &self.atoms.dynamic
    }

    #[must_use]
    pub(super) const fn nodes(&self) -> &[ImageNode] {
        &self.nodes
    }

    /// Project SAB occurrences solely while the transport finalizer binds this
    /// image to its authenticated backing descriptor table.
    ///
    /// The completed transport archive exposes no corresponding image or node
    /// accessor, so archive-local backing IDs cannot be detached from the table
    /// which gives them meaning.
    pub(in crate::runtime::binary_object) fn sab_archive_occurrences(
        &self,
    ) -> impl Iterator<Item = SabArchiveOccurrence> + '_ {
        self.nodes.iter().filter_map(|node| match node {
            WireNodeCarrier::SharedArrayBuffer {
                byte_length,
                max_byte_length,
                backing,
            } => Some(SabArchiveOccurrence::new(
                *byte_length,
                *max_byte_length,
                *backing,
            )),
            _ => None,
        })
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
    pub(in crate::runtime) const fn modules(&self) -> &[ModuleRecord] {
        &self.modules
    }

    #[must_use]
    pub(in crate::runtime) fn module(&self, id: ModuleId) -> Option<&ModuleRecord> {
        (id.source() == self.source)
            .then(|| self.modules.get(id.as_usize()))
            .flatten()
    }

    #[must_use]
    pub(in crate::runtime) const fn root(&self) -> &ImageValue {
        &self.root
    }
}

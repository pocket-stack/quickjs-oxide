//! Data model for a non-executable FunctionBytecode record prefix.

use super::super::atoms::{AtomIndexSpace, BinaryAtom};
use super::super::code::CodeImage;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(in crate::runtime) enum FunctionKind {
    Normal = 0,
    Generator = 1,
    Async = 2,
    AsyncGenerator = 3,
}

impl FunctionKind {
    const fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::Normal,
            1 => Self::Generator,
            2 => Self::Async,
            3 => Self::AsyncGenerator,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionFlags(pub(super) u16);

impl FunctionFlags {
    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime) const fn has_prototype(self) -> bool {
        self.0 & (1 << 0) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn has_simple_parameter_list(self) -> bool {
        self.0 & (1 << 1) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_derived_class_constructor(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn needs_home_object(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn kind(self) -> FunctionKind {
        FunctionKind::from_bits((self.0 >> 4) & 0b11)
    }

    #[must_use]
    pub(in crate::runtime) const fn allows_new_target(self) -> bool {
        self.0 & (1 << 6) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn allows_super_call(self) -> bool {
        self.0 & (1 << 7) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn allows_super_property(self) -> bool {
        self.0 & (1 << 8) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn allows_arguments(self) -> bool {
        self.0 & (1 << 9) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_direct_or_indirect_eval(self) -> bool {
        self.0 & (1 << 11) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct JsMode(pub(super) u8);

impl JsMode {
    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_strict(self) -> bool {
        self.0 & (1 << 0) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_async(self) -> bool {
        self.0 & (1 << 2) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_backtrace_barrier(self) -> bool {
        self.0 & (1 << 3) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct VariableKind(pub(super) u8);

impl VariableKind {
    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct LocalVariableFlags(pub(super) u8);

impl LocalVariableFlags {
    pub(super) const fn decode(raw: u8) -> Self {
        Self(raw)
    }

    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime) const fn kind(self) -> VariableKind {
        VariableKind(self.0 & 0x0f)
    }

    #[must_use]
    pub(in crate::runtime) const fn is_const(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_lexical(self) -> bool {
        self.0 & (1 << 5) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_captured(self) -> bool {
        self.0 & (1 << 6) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn has_scope(self) -> bool {
        self.0 & (1 << 7) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(in crate::runtime) enum ClosureType {
    Local = 0,
    Argument = 1,
    Reference = 2,
    GlobalReference = 3,
    GlobalDeclaration = 4,
    Global = 5,
    ModuleDeclaration = 6,
    ModuleImport = 7,
}

impl ClosureType {
    const fn from_bits(bits: u16) -> Self {
        match bits {
            0 => Self::Local,
            1 => Self::Argument,
            2 => Self::Reference,
            3 => Self::GlobalReference,
            4 => Self::GlobalDeclaration,
            5 => Self::Global,
            6 => Self::ModuleDeclaration,
            7 => Self::ModuleImport,
            _ => unreachable!(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ClosureVariableFlags(pub(super) u16);

impl ClosureVariableFlags {
    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u16 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime) const fn closure_type(self) -> ClosureType {
        ClosureType::from_bits(self.0 & 0b111)
    }

    #[must_use]
    pub(in crate::runtime) const fn is_const(self) -> bool {
        self.0 & (1 << 3) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn is_lexical(self) -> bool {
        self.0 & (1 << 4) != 0
    }

    #[must_use]
    pub(in crate::runtime) const fn kind(self) -> VariableKind {
        VariableKind(((self.0 >> 5) & 0x0f) as u8)
    }
}

/// Signed QuickJS local-scope link after decoding the wire's plus-one form.
///
/// In particular, the argument-scope terminator is -2 and is spelled as
/// u32::MAX on the wire. The one encoding that would decrement i32::MIN is
/// rejected instead of reproducing C signed-overflow undefined behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ScopeLink(pub(super) i32);

impl ScopeLink {
    #[must_use]
    pub(in crate::runtime) const fn value(self) -> i32 {
        self.0
    }

    pub(in crate::runtime::binary_object) const fn encode(self) -> Option<u32> {
        match self.0.checked_add(1) {
            Some(value) => Some(value as u32),
            None => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct LocalVariableImage {
    pub(super) name: BinaryAtom,
    pub(super) scope_next: ScopeLink,
    pub(super) variable_reference_index: u16,
    pub(super) flags: LocalVariableFlags,
}

impl LocalVariableImage {
    #[must_use]
    pub(in crate::runtime) const fn name(&self) -> BinaryAtom {
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

    pub(in crate::runtime::binary_object) fn into_parts(
        self,
    ) -> (BinaryAtom, ScopeLink, u16, LocalVariableFlags) {
        (
            self.name,
            self.scope_next,
            self.variable_reference_index,
            self.flags,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ClosureVariableImage {
    pub(super) name: BinaryAtom,
    pub(super) variable_index: u16,
    pub(super) flags: ClosureVariableFlags,
}

impl ClosureVariableImage {
    #[must_use]
    pub(in crate::runtime) const fn name(&self) -> BinaryAtom {
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

    pub(in crate::runtime::binary_object) fn into_parts(
        self,
    ) -> (BinaryAtom, u16, ClosureVariableFlags) {
        (self.name, self.variable_index, self.flags)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionDebugImage {
    pub(super) filename: BinaryAtom,
    pub(super) pc2line: Box<[u8]>,
    pub(super) source: Box<[u8]>,
}

impl FunctionDebugImage {
    #[must_use]
    pub(in crate::runtime) const fn filename(&self) -> BinaryAtom {
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

    pub(in crate::runtime::binary_object) fn into_parts(
        self,
    ) -> (BinaryAtom, Box<[u8]>, Box<[u8]>) {
        (self.filename, self.pc2line, self.source)
    }
}

/// The complete fixed metadata of one FunctionBytecode occurrence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionEnvelope {
    pub(super) atom_space: AtomIndexSpace,
    pub(super) flags: FunctionFlags,
    pub(super) js_mode: JsMode,
    pub(super) name: BinaryAtom,
    pub(super) argument_count: u16,
    pub(super) variable_count: u16,
    pub(super) defined_argument_count: u16,
    pub(super) stack_size: u16,
    pub(super) variable_reference_count: u16,
    pub(super) locals: Box<[LocalVariableImage]>,
    pub(super) closures: Box<[ClosureVariableImage]>,
    pub(super) code: CodeImage,
    pub(super) debug: Option<FunctionDebugImage>,
}

/// Linear handoff from the raw function-prefix model to semantic image
/// relocation.
pub(in crate::runtime::binary_object) struct FunctionEnvelopeParts {
    pub(in crate::runtime::binary_object) atom_space: AtomIndexSpace,
    pub(in crate::runtime::binary_object) flags: FunctionFlags,
    pub(in crate::runtime::binary_object) js_mode: JsMode,
    pub(in crate::runtime::binary_object) name: BinaryAtom,
    pub(in crate::runtime::binary_object) argument_count: u16,
    pub(in crate::runtime::binary_object) variable_count: u16,
    pub(in crate::runtime::binary_object) defined_argument_count: u16,
    pub(in crate::runtime::binary_object) stack_size: u16,
    pub(in crate::runtime::binary_object) variable_reference_count: u16,
    pub(in crate::runtime::binary_object) locals: Box<[LocalVariableImage]>,
    pub(in crate::runtime::binary_object) closures: Box<[ClosureVariableImage]>,
    pub(in crate::runtime::binary_object) code: CodeImage,
    pub(in crate::runtime::binary_object) debug: Option<FunctionDebugImage>,
}

impl FunctionEnvelope {
    #[must_use]
    pub(in crate::runtime) const fn flags(&self) -> FunctionFlags {
        self.flags
    }

    #[must_use]
    pub(in crate::runtime) const fn js_mode(&self) -> JsMode {
        self.js_mode
    }

    #[must_use]
    pub(in crate::runtime) const fn name(&self) -> BinaryAtom {
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
    pub(in crate::runtime) const fn locals(&self) -> &[LocalVariableImage] {
        &self.locals
    }

    #[must_use]
    pub(in crate::runtime) const fn closures(&self) -> &[ClosureVariableImage] {
        &self.closures
    }

    #[must_use]
    pub(in crate::runtime) const fn code(&self) -> &CodeImage {
        &self.code
    }

    #[must_use]
    pub(in crate::runtime) const fn debug(&self) -> Option<&FunctionDebugImage> {
        self.debug.as_ref()
    }

    pub(in crate::runtime::binary_object) fn into_parts(self) -> FunctionEnvelopeParts {
        FunctionEnvelopeParts {
            atom_space: self.atom_space,
            flags: self.flags,
            js_mode: self.js_mode,
            name: self.name,
            argument_count: self.argument_count,
            variable_count: self.variable_count,
            defined_argument_count: self.defined_argument_count,
            stack_size: self.stack_size,
            variable_reference_count: self.variable_reference_count,
            locals: self.locals,
            closures: self.closures,
            code: self.code,
            debug: self.debug,
        }
    }
}

/// A parsed fixed envelope plus the number of constant-pool values still
/// pending on the shared whole-image cursor.
///
/// Keeping this count outside FunctionEnvelope makes an incomplete record
/// impossible to confuse with the future fully decoded FunctionImage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionRecordPrefix {
    pub(super) envelope: FunctionEnvelope,
    pub(super) pending_constant_pool_count: u32,
}

impl FunctionRecordPrefix {
    #[must_use]
    pub(in crate::runtime) const fn envelope(&self) -> &FunctionEnvelope {
        &self.envelope
    }

    #[must_use]
    pub(in crate::runtime) const fn pending_constant_pool_count(&self) -> u32 {
        self.pending_constant_pool_count
    }

    pub(in crate::runtime::binary_object) fn into_parts(self) -> (FunctionEnvelope, u32) {
        (self.envelope, self.pending_constant_pool_count)
    }
}

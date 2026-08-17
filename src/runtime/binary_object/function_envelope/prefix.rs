//! Bounded reader and prefix writer for the fixed FunctionBytecode envelope.

use std::fmt;

use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::code::{CodeError, CodeImage, CodeLimits, CodeResourceKind};
use super::super::read_cursor::CheckedReadCursor;
use super::super::wire::{ReaderMode, WireError, WireWriter};
use super::model::*;

const FUNCTION_FLAGS_MASK: u16 = 0x0fff;
const FUNCTION_HAS_DEBUG: u16 = 1 << 10;
const CLOSURE_FLAGS_MASK: u16 = 0x01ff;
const MAX_QUICKJS_POSITIVE_INT: u32 = i32::MAX as u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionEnvelopeLimits {
    max_local_variables: usize,
    max_closure_variables: usize,
    max_constant_pool_entries: usize,
    max_pc2line_bytes: usize,
    max_source_bytes: usize,
    max_total_debug_bytes: usize,
    code_limits: CodeLimits,
}

impl FunctionEnvelopeLimits {
    #[must_use]
    pub(in crate::runtime) const fn new(
        max_local_variables: usize,
        max_closure_variables: usize,
        max_constant_pool_entries: usize,
        max_pc2line_bytes: usize,
        max_source_bytes: usize,
        max_total_debug_bytes: usize,
        code_limits: CodeLimits,
    ) -> Self {
        Self {
            max_local_variables,
            max_closure_variables,
            max_constant_pool_entries,
            max_pc2line_bytes,
            max_source_bytes,
            max_total_debug_bytes,
            code_limits,
        }
    }

    /// Intersect one envelope's limits with the remaining whole-image budget.
    ///
    /// The individual pc2line/source caps remain per-function. Their combined
    /// cap is intersected because the prefix checks it before copying either
    /// debug payload.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn intersect_counts(
        self,
        max_local_variables: usize,
        max_closure_variables: usize,
        max_constant_pool_entries: usize,
        max_total_debug_bytes: usize,
    ) -> Self {
        Self {
            max_local_variables: minimum(self.max_local_variables, max_local_variables),
            max_closure_variables: minimum(self.max_closure_variables, max_closure_variables),
            max_constant_pool_entries: minimum(
                self.max_constant_pool_entries,
                max_constant_pool_entries,
            ),
            max_pc2line_bytes: self.max_pc2line_bytes,
            max_source_bytes: self.max_source_bytes,
            max_total_debug_bytes: minimum(self.max_total_debug_bytes, max_total_debug_bytes),
            code_limits: self.code_limits,
        }
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn intersect_code(
        self,
        max_bytes: usize,
        max_instructions: usize,
        max_atom_relocations: usize,
    ) -> Self {
        Self {
            code_limits: self.code_limits.intersect(
                max_bytes,
                max_instructions,
                max_atom_relocations,
            ),
            ..self
        }
    }

    pub(in crate::runtime::binary_object) const fn limit(
        self,
        kind: FunctionResourceKind,
    ) -> usize {
        match kind {
            FunctionResourceKind::LocalVariables => self.max_local_variables,
            FunctionResourceKind::ClosureVariables => self.max_closure_variables,
            FunctionResourceKind::ConstantPoolEntries => self.max_constant_pool_entries,
            FunctionResourceKind::Pc2LineBytes => self.max_pc2line_bytes,
            FunctionResourceKind::SourceBytes => self.max_source_bytes,
            FunctionResourceKind::TotalDebugBytes => self.max_total_debug_bytes,
        }
    }

    pub(in crate::runtime::binary_object) const fn code_limit(
        self,
        kind: super::super::code::CodeResourceKind,
    ) -> usize {
        self.code_limits.limit(kind)
    }

    fn check(
        self,
        kind: FunctionResourceKind,
        requested: usize,
    ) -> Result<(), FunctionEnvelopeError> {
        let limit = self.limit(kind);
        if requested > limit {
            return Err(FunctionEnvelopeError::ResourceLimit {
                kind,
                requested,
                limit,
            });
        }
        Ok(())
    }
}

const fn minimum(left: usize, right: usize) -> usize {
    if left < right { left } else { right }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FunctionResourceKind {
    LocalVariables,
    ClosureVariables,
    ConstantPoolEntries,
    Pc2LineBytes,
    SourceBytes,
    TotalDebugBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FunctionField {
    FunctionFlags,
    ArgumentCount,
    VariableCount,
    DefinedArgumentCount,
    StackSize,
    VariableReferenceCount,
    ClosureVariableCount,
    ConstantPoolCount,
    ByteCodeLength,
    LocalCount,
    LocalScopeNext,
    LocalVariableReferenceIndex,
    ClosureVariableIndex,
    ClosureFlags,
    Pc2LineLength,
    SourceLength,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FunctionEnvelopeError {
    Wire(WireError),
    Code(CodeError),
    InvalidAtomMode {
        found: BinaryObjectMode,
    },
    FieldOutOfRange {
        field: FunctionField,
        offset: usize,
        value: u32,
        maximum: u32,
    },
    ReservedBits {
        field: FunctionField,
        offset: usize,
        bits: u16,
    },
    InvalidModelBits {
        field: FunctionField,
        bits: u16,
    },
    InvalidModelAtom {
        atom: BinaryAtom,
        atom_space: AtomIndexSpace,
    },
    MismatchedAtomSpace {
        envelope: AtomIndexSpace,
        code: AtomIndexSpace,
    },
    InvalidScopeEncoding {
        offset: usize,
        encoded: u32,
    },
    NonCanonicalLocalTableLength {
        argument_count: u16,
        variable_count: u16,
        local_count: usize,
    },
    ResourceLimit {
        kind: FunctionResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        field: FunctionField,
    },
    AllocationFailed,
}

impl fmt::Display for FunctionEnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Code(error) => fmt::Display::fmt(error, formatter),
            Self::InvalidAtomMode { found } => write!(
                formatter,
                "function envelope requires the bytecode atom namespace, found {found:?}"
            ),
            Self::FieldOutOfRange {
                field,
                offset,
                value,
                maximum,
            } => write!(
                formatter,
                "{field:?} value {value} at byte {offset} exceeds {maximum}"
            ),
            Self::ReservedBits {
                field,
                offset,
                bits,
            } => write!(
                formatter,
                "{field:?} has reserved bits 0x{bits:04x} at byte {offset}"
            ),
            Self::InvalidModelBits { field, bits } => write!(
                formatter,
                "{field:?} contains non-semantic model bits 0x{bits:04x}"
            ),
            Self::InvalidModelAtom { atom, atom_space } => write!(
                formatter,
                "metadata atom {atom:?} does not belong to function namespace {atom_space:?}"
            ),
            Self::MismatchedAtomSpace { envelope, code } => write!(
                formatter,
                "function envelope namespace {envelope:?} does not match code namespace {code:?}"
            ),
            Self::InvalidScopeEncoding { offset, encoded } => write!(
                formatter,
                "local scope link 0x{encoded:08x} at byte {offset} would overflow QuickJS signed decrement"
            ),
            Self::NonCanonicalLocalTableLength {
                argument_count,
                variable_count,
                local_count,
            } => write!(
                formatter,
                "local table length {local_count} is neither zero nor arg_count {argument_count} + var_count {variable_count}"
            ),
            Self::ResourceLimit {
                kind,
                requested,
                limit,
            } => write!(
                formatter,
                "{kind:?} function-envelope resource limit exceeded: requested {requested}, limit {limit}"
            ),
            Self::CountOverflow { field } => {
                write!(formatter, "{field:?} cannot be represented on the BC5 wire")
            }
            Self::AllocationFailed => formatter.write_str("function-envelope allocation failed"),
        }
    }
}

impl std::error::Error for FunctionEnvelopeError {}

impl From<WireError> for FunctionEnvelopeError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<CodeError> for FunctionEnvelopeError {
    fn from(error: CodeError) -> Self {
        Self::Code(error)
    }
}

/// Decode the fixed function body after its tag has already been consumed.
///
/// The cursor stops immediately before the first constant-pool value. A caller
/// must keep using the same whole-image decode state for those values; calling
/// the data-only graph decoder per entry would corrupt object-reference
/// numbering across nested functions.
pub(in crate::runtime) fn read_function_record_prefix_after_tag<'input, C>(
    cursor: &mut C,
    atom_space: AtomIndexSpace,
    limits: FunctionEnvelopeLimits,
) -> Result<FunctionRecordPrefix, FunctionEnvelopeError>
where
    C: CheckedReadCursor<'input>,
{
    if atom_space.mode() != BinaryObjectMode::Bytecode {
        return Err(FunctionEnvelopeError::InvalidAtomMode {
            found: atom_space.mode(),
        });
    }
    let flags_offset = cursor.position();
    let (flags, has_debug) =
        decode_function_flags(cursor.read_u16_le()?, cursor.mode(), flags_offset)?;
    let js_mode = JsMode(cursor.read_u8()?);
    let name = atom_space.decode_metadata_atom(cursor)?;

    let argument_count = read_truncating_u16(cursor, FunctionField::ArgumentCount)?;
    let variable_count = read_truncating_u16(cursor, FunctionField::VariableCount)?;
    let defined_argument_count = read_truncating_u16(cursor, FunctionField::DefinedArgumentCount)?;
    let stack_size = read_truncating_u16(cursor, FunctionField::StackSize)?;
    let variable_reference_count =
        read_truncating_u16(cursor, FunctionField::VariableReferenceCount)?;

    let closure_count = read_positive_int(cursor, FunctionField::ClosureVariableCount)?;
    let constant_pool_count = read_positive_int(cursor, FunctionField::ConstantPoolCount)?;
    let byte_code_length = read_positive_int(cursor, FunctionField::ByteCodeLength)?;
    let local_count = read_positive_int(cursor, FunctionField::LocalCount)?;
    if cursor.mode() == ReaderMode::Strict
        && !is_canonical_local_count(argument_count, variable_count, local_count)
    {
        return Err(FunctionEnvelopeError::NonCanonicalLocalTableLength {
            argument_count,
            variable_count,
            local_count,
        });
    }

    limits.check(FunctionResourceKind::LocalVariables, local_count)?;
    limits.check(FunctionResourceKind::ClosureVariables, closure_count)?;
    limits.check(
        FunctionResourceKind::ConstantPoolEntries,
        constant_pool_count,
    )?;
    // The byte length is already known from the prefix. Enforce it before
    // reading local/closure tables or slicing/copying the code payload so a
    // whole-image remaining budget is a rejection-time work boundary.
    limits
        .code_limits
        .check(CodeResourceKind::Bytes, byte_code_length)?;

    let mut locals = Vec::new();
    locals
        .try_reserve_exact(local_count)
        .map_err(|_| FunctionEnvelopeError::AllocationFailed)?;
    for _ in 0..local_count {
        let name = atom_space.decode_metadata_atom(cursor)?;
        let scope_offset = cursor.position();
        let scope_next = decode_scope_link(cursor.read_uleb128()?, scope_offset)?;
        let variable_reference_index =
            read_truncating_u16(cursor, FunctionField::LocalVariableReferenceIndex)?;
        let flags = LocalVariableFlags::decode(cursor.read_u8()?);
        locals.push(LocalVariableImage {
            name,
            scope_next,
            variable_reference_index,
            flags,
        });
    }

    let mut closures = Vec::new();
    closures
        .try_reserve_exact(closure_count)
        .map_err(|_| FunctionEnvelopeError::AllocationFailed)?;
    for _ in 0..closure_count {
        let name = atom_space.decode_metadata_atom(cursor)?;
        let variable_index = read_truncating_u16(cursor, FunctionField::ClosureVariableIndex)?;
        let flags_offset = cursor.position();
        let flags = decode_closure_flags(cursor.read_u16_le()?, cursor.mode(), flags_offset)?;
        closures.push(ClosureVariableImage {
            name,
            variable_index,
            flags,
        });
    }

    let code_offset = cursor.position();
    let code_bytes = cursor.read_bytes(byte_code_length)?;
    let code = CodeImage::scan(code_bytes, atom_space, code_offset, limits.code_limits)?;

    let debug = has_debug
        .then(|| read_debug_image(cursor, atom_space, limits))
        .transpose()?;

    Ok(FunctionRecordPrefix {
        envelope: FunctionEnvelope {
            atom_space,
            flags,
            js_mode,
            name,
            argument_count,
            variable_count,
            defined_argument_count,
            stack_size,
            variable_reference_count,
            locals: locals.into_boxed_slice(),
            closures: closures.into_boxed_slice(),
            code,
            debug,
        },
        pending_constant_pool_count: constant_pool_count as u32,
    })
}

/// Write the fixed function body after tag 12, but not its constant-pool values.
///
/// This is intentionally named a prefix writer. Exactly
/// pending_constant_pool_count() recursively encoded values must be appended by a
/// future whole-image encoder that owns the shared object-reference table.
pub(in crate::runtime) fn write_function_record_prefix_after_tag(
    prefix: &FunctionRecordPrefix,
    writer: &mut WireWriter,
) -> Result<(), FunctionEnvelopeError> {
    let envelope = &prefix.envelope;
    if envelope.atom_space.mode() != BinaryObjectMode::Bytecode {
        return Err(FunctionEnvelopeError::InvalidAtomMode {
            found: envelope.atom_space.mode(),
        });
    }
    if envelope.code.atom_space() != envelope.atom_space {
        return Err(FunctionEnvelopeError::MismatchedAtomSpace {
            envelope: envelope.atom_space,
            code: envelope.code.atom_space(),
        });
    }
    let invalid_function_bits = envelope.flags.raw() & (!FUNCTION_FLAGS_MASK | FUNCTION_HAS_DEBUG);
    if invalid_function_bits != 0 {
        return Err(FunctionEnvelopeError::InvalidModelBits {
            field: FunctionField::FunctionFlags,
            bits: invalid_function_bits,
        });
    }
    for closure in &envelope.closures {
        let invalid_closure_bits = closure.flags.raw() & !CLOSURE_FLAGS_MASK;
        if invalid_closure_bits != 0 {
            return Err(FunctionEnvelopeError::InvalidModelBits {
                field: FunctionField::ClosureFlags,
                bits: invalid_closure_bits,
            });
        }
    }
    if envelope
        .locals
        .iter()
        .any(|local| local.scope_next.encode().is_none())
    {
        return Err(FunctionEnvelopeError::CountOverflow {
            field: FunctionField::LocalScopeNext,
        });
    }
    validate_model_atom(envelope.atom_space, envelope.name)?;
    for local in &envelope.locals {
        validate_model_atom(envelope.atom_space, local.name)?;
    }
    for closure in &envelope.closures {
        validate_model_atom(envelope.atom_space, closure.name)?;
    }
    if let Some(debug) = &envelope.debug {
        validate_model_atom(envelope.atom_space, debug.filename)?;
    }
    if !is_canonical_local_count(
        envelope.argument_count,
        envelope.variable_count,
        envelope.locals.len(),
    ) {
        return Err(FunctionEnvelopeError::NonCanonicalLocalTableLength {
            argument_count: envelope.argument_count,
            variable_count: envelope.variable_count,
            local_count: envelope.locals.len(),
        });
    }
    if prefix.pending_constant_pool_count > MAX_QUICKJS_POSITIVE_INT {
        return Err(FunctionEnvelopeError::CountOverflow {
            field: FunctionField::ConstantPoolCount,
        });
    }
    let closure_count =
        positive_wire_count(envelope.closures.len(), FunctionField::ClosureVariableCount)?;
    let local_count = positive_wire_count(envelope.locals.len(), FunctionField::LocalCount)?;
    let code_bytes = envelope.code.canonical_bytes()?;
    let code_length = positive_wire_count(code_bytes.len(), FunctionField::ByteCodeLength)?;
    let pc2line_length = envelope.debug.as_ref().map_or(Ok(0), |debug| {
        positive_wire_count(debug.pc2line.len(), FunctionField::Pc2LineLength)
    })?;
    let source_length = envelope.debug.as_ref().map_or(Ok(0), |debug| {
        positive_wire_count(debug.source.len(), FunctionField::SourceLength)
    })?;

    let function_flags =
        envelope.flags.raw() | (u16::from(envelope.debug.is_some()) * FUNCTION_HAS_DEBUG);
    writer.write_u16_le(function_flags)?;
    writer.write_u8(envelope.js_mode.raw())?;
    envelope
        .atom_space
        .encode_metadata_atom(writer, envelope.name)?;
    writer.write_uleb128(u32::from(envelope.argument_count))?;
    writer.write_uleb128(u32::from(envelope.variable_count))?;
    writer.write_uleb128(u32::from(envelope.defined_argument_count))?;
    writer.write_uleb128(u32::from(envelope.stack_size))?;
    writer.write_uleb128(u32::from(envelope.variable_reference_count))?;
    writer.write_uleb128(closure_count)?;
    writer.write_uleb128(prefix.pending_constant_pool_count)?;
    writer.write_uleb128(code_length)?;
    writer.write_uleb128(local_count)?;

    for local in &envelope.locals {
        envelope
            .atom_space
            .encode_metadata_atom(writer, local.name)?;
        let encoded_scope =
            local
                .scope_next
                .encode()
                .ok_or(FunctionEnvelopeError::CountOverflow {
                    field: FunctionField::LocalScopeNext,
                })?;
        writer.write_uleb128(encoded_scope)?;
        writer.write_uleb128(u32::from(local.variable_reference_index))?;
        writer.write_u8(local.flags.raw())?;
    }
    for closure in &envelope.closures {
        envelope
            .atom_space
            .encode_metadata_atom(writer, closure.name)?;
        writer.write_uleb128(u32::from(closure.variable_index))?;
        writer.write_u16_le(closure.flags.raw())?;
    }

    writer.write_bytes(&code_bytes)?;
    if let Some(debug) = &envelope.debug {
        envelope
            .atom_space
            .encode_metadata_atom(writer, debug.filename)?;
        writer.write_uleb128(pc2line_length)?;
        writer.write_bytes(&debug.pc2line)?;
        writer.write_uleb128(source_length)?;
        writer.write_bytes(&debug.source)?;
    }
    Ok(())
}

fn validate_model_atom(
    atom_space: AtomIndexSpace,
    atom: BinaryAtom,
) -> Result<(), FunctionEnvelopeError> {
    atom_space
        .validate_metadata_atom(atom, 0)
        .map_err(|_| FunctionEnvelopeError::InvalidModelAtom { atom, atom_space })
}

fn decode_function_flags(
    raw: u16,
    mode: ReaderMode,
    offset: usize,
) -> Result<(FunctionFlags, bool), FunctionEnvelopeError> {
    let reserved = raw & !FUNCTION_FLAGS_MASK;
    if mode == ReaderMode::Strict && reserved != 0 {
        return Err(FunctionEnvelopeError::ReservedBits {
            field: FunctionField::FunctionFlags,
            offset,
            bits: reserved,
        });
    }
    Ok((
        FunctionFlags(raw & FUNCTION_FLAGS_MASK & !FUNCTION_HAS_DEBUG),
        raw & FUNCTION_HAS_DEBUG != 0,
    ))
}

fn decode_closure_flags(
    raw: u16,
    mode: ReaderMode,
    offset: usize,
) -> Result<ClosureVariableFlags, FunctionEnvelopeError> {
    let reserved = raw & !CLOSURE_FLAGS_MASK;
    if mode == ReaderMode::Strict && reserved != 0 {
        return Err(FunctionEnvelopeError::ReservedBits {
            field: FunctionField::ClosureFlags,
            offset,
            bits: reserved,
        });
    }
    Ok(ClosureVariableFlags(raw & CLOSURE_FLAGS_MASK))
}

fn decode_scope_link(encoded: u32, offset: usize) -> Result<ScopeLink, FunctionEnvelopeError> {
    let plus_one = encoded as i32;
    let Some(scope_next) = plus_one.checked_sub(1) else {
        return Err(FunctionEnvelopeError::InvalidScopeEncoding { offset, encoded });
    };
    Ok(ScopeLink(scope_next))
}

fn read_truncating_u16<'input, C>(
    cursor: &mut C,
    field: FunctionField,
) -> Result<u16, FunctionEnvelopeError>
where
    C: CheckedReadCursor<'input>,
{
    let offset = cursor.position();
    let value = cursor.read_uleb128()?;
    if cursor.mode() == ReaderMode::Strict && value > u32::from(u16::MAX) {
        return Err(FunctionEnvelopeError::FieldOutOfRange {
            field,
            offset,
            value,
            maximum: u32::from(u16::MAX),
        });
    }
    // Pinned QuickJS assigns the u32 decoder result directly into u16 storage.
    Ok(value as u16)
}

fn read_positive_int<'input, C>(
    cursor: &mut C,
    field: FunctionField,
) -> Result<usize, FunctionEnvelopeError>
where
    C: CheckedReadCursor<'input>,
{
    let offset = cursor.position();
    let value = cursor.read_uleb128()?;
    if value > MAX_QUICKJS_POSITIVE_INT {
        return Err(FunctionEnvelopeError::FieldOutOfRange {
            field,
            offset,
            value,
            maximum: MAX_QUICKJS_POSITIVE_INT,
        });
    }
    Ok(value as usize)
}

fn read_debug_image<'input, C>(
    cursor: &mut C,
    atom_space: AtomIndexSpace,
    limits: FunctionEnvelopeLimits,
) -> Result<FunctionDebugImage, FunctionEnvelopeError>
where
    C: CheckedReadCursor<'input>,
{
    let filename = atom_space.decode_metadata_atom(cursor)?;
    let pc2line_length = read_positive_int(cursor, FunctionField::Pc2LineLength)?;
    limits.check(FunctionResourceKind::Pc2LineBytes, pc2line_length)?;
    let pc2line_bytes = cursor.read_bytes(pc2line_length)?;

    let source_length = read_positive_int(cursor, FunctionField::SourceLength)?;
    limits.check(FunctionResourceKind::SourceBytes, source_length)?;
    let total_debug_bytes =
        pc2line_length
            .checked_add(source_length)
            .ok_or(FunctionEnvelopeError::CountOverflow {
                field: FunctionField::SourceLength,
            })?;
    limits.check(FunctionResourceKind::TotalDebugBytes, total_debug_bytes)?;
    let source_bytes = cursor.read_bytes(source_length)?;
    let pc2line = copy_bytes(pc2line_bytes)?;
    let source = copy_bytes(source_bytes)?;

    Ok(FunctionDebugImage {
        filename,
        pc2line,
        source,
    })
}

fn copy_bytes(bytes: &[u8]) -> Result<Box<[u8]>, FunctionEnvelopeError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes.len())
        .map_err(|_| FunctionEnvelopeError::AllocationFailed)?;
    output.extend_from_slice(bytes);
    Ok(output.into_boxed_slice())
}

fn positive_wire_count(value: usize, field: FunctionField) -> Result<u32, FunctionEnvelopeError> {
    let value = u32::try_from(value).map_err(|_| FunctionEnvelopeError::CountOverflow { field })?;
    if value > MAX_QUICKJS_POSITIVE_INT {
        return Err(FunctionEnvelopeError::CountOverflow { field });
    }
    Ok(value)
}

const fn is_canonical_local_count(
    argument_count: u16,
    variable_count: u16,
    local_count: usize,
) -> bool {
    local_count == 0 || local_count == argument_count as usize + variable_count as usize
}

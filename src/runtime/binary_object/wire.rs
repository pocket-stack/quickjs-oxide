//! Pure wire primitives for QuickJS 2026-06-04 binary objects.
//!
//! This is not a persistence format. It is a checked, release-pinned
//! implementation of the `BC_VERSION == 5` framing needed for later BJSON and
//! bytecode interoperability. Nothing in this module touches a runtime heap.

use std::fmt;

/// Binary-object version used by the pinned QuickJS 2026-06-04 release.
pub(in crate::runtime) const BC_VERSION: u8 = 5;

/// QuickJS stores string lengths in 30 bits even though the wire length field
/// itself can spell larger values.
pub(in crate::runtime) const MAX_STRING_CODE_UNITS: usize = (1 << 30) - 1;

/// Exact on-wire value tags from QuickJS `BCTagEnum`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(in crate::runtime) enum BcTag {
    Null = 1,
    Undefined = 2,
    BoolFalse = 3,
    BoolTrue = 4,
    Int32 = 5,
    Float64 = 6,
    String = 7,
    Object = 8,
    Array = 9,
    BigInt = 10,
    TemplateObject = 11,
    FunctionBytecode = 12,
    Module = 13,
    TypedArray = 14,
    ArrayBuffer = 15,
    SharedArrayBuffer = 16,
    Date = 17,
    ObjectValue = 18,
    ObjectReference = 19,
}

impl BcTag {
    #[must_use]
    pub(in crate::runtime) const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Null),
            2 => Some(Self::Undefined),
            3 => Some(Self::BoolFalse),
            4 => Some(Self::BoolTrue),
            5 => Some(Self::Int32),
            6 => Some(Self::Float64),
            7 => Some(Self::String),
            8 => Some(Self::Object),
            9 => Some(Self::Array),
            10 => Some(Self::BigInt),
            11 => Some(Self::TemplateObject),
            12 => Some(Self::FunctionBytecode),
            13 => Some(Self::Module),
            14 => Some(Self::TypedArray),
            15 => Some(Self::ArrayBuffer),
            16 => Some(Self::SharedArrayBuffer),
            17 => Some(Self::Date),
            18 => Some(Self::ObjectValue),
            19 => Some(Self::ObjectReference),
            _ => None,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn to_byte(self) -> u8 {
        self as u8
    }
}

/// Compatibility controls that do not weaken local resource limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ReaderMode {
    /// Require canonical ULEB128 encodings and full input consumption.
    Strict,
    /// Match the pinned QuickJS reader's acceptance of non-minimal ULEB128
    /// values and trailing bytes.
    QuickJsCompatible,
}

/// Explicit allocation and traversal limits for one wire reader.
///
/// There is intentionally no `Default`: a future caller must choose limits for
/// its own trust boundary rather than silently inheriting an oracle-oriented
/// policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct WireLimits {
    max_input_bytes: usize,
    max_atom_count: u32,
    max_string_code_units: usize,
    max_total_string_code_units: usize,
}

impl WireLimits {
    #[must_use]
    pub(in crate::runtime) const fn new(
        max_input_bytes: usize,
        max_atom_count: u32,
        max_string_code_units: usize,
        max_total_string_code_units: usize,
    ) -> Self {
        Self {
            max_input_bytes,
            max_atom_count,
            max_string_code_units,
            max_total_string_code_units,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ResourceKind {
    InputBytes,
    OutputBytes,
    AtomCount,
    StringCodeUnits,
    TotalStringCodeUnits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum WireError {
    ResourceLimit {
        kind: ResourceKind,
        requested: usize,
        limit: usize,
    },
    Truncated {
        offset: usize,
        needed: usize,
        remaining: usize,
    },
    LengthOverflow {
        offset: usize,
    },
    MalformedUleb128 {
        offset: usize,
    },
    NonCanonicalUleb128 {
        offset: usize,
    },
    AtomIndexSpaceOverflow {
        first_atom: u32,
        atom_count: u32,
        maximum: u32,
    },
    InvalidAtomIndex {
        offset: usize,
        index: u32,
        first_atom: u32,
        atom_count: u32,
    },
    InvalidVersion {
        found: u8,
        expected: u8,
    },
    InvalidTag {
        tag: u8,
        offset: usize,
    },
    StringTooLong {
        offset: usize,
        length: usize,
        maximum: usize,
    },
    TrailingBytes {
        offset: usize,
        remaining: usize,
    },
    AllocationFailed,
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimit {
                kind,
                requested,
                limit,
            } => write!(
                formatter,
                "{kind:?} resource limit exceeded: requested {requested}, limit {limit}"
            ),
            Self::Truncated {
                offset,
                needed,
                remaining,
            } => write!(
                formatter,
                "binary object truncated at byte {offset}: needed {needed}, remaining {remaining}"
            ),
            Self::LengthOverflow { offset } => {
                write!(formatter, "binary object length overflow at byte {offset}")
            }
            Self::MalformedUleb128 { offset } => {
                write!(formatter, "malformed ULEB128 at byte {offset}")
            }
            Self::NonCanonicalUleb128 { offset } => {
                write!(formatter, "non-canonical ULEB128 at byte {offset}")
            }
            Self::AtomIndexSpaceOverflow {
                first_atom,
                atom_count,
                maximum,
            } => write!(
                formatter,
                "atom index space starting at {first_atom} with {atom_count} header atoms exceeds maximum table index {maximum}"
            ),
            Self::InvalidAtomIndex {
                offset,
                index,
                first_atom,
                atom_count,
            } => write!(
                formatter,
                "invalid atom index {index} at byte {offset} (first atom {first_atom}, atom count {atom_count})"
            ),
            Self::InvalidVersion { found, expected } => {
                write!(formatter, "invalid version ({found} expected={expected})")
            }
            Self::InvalidTag { tag, offset } => {
                write!(formatter, "invalid tag (tag={tag} pos={offset})")
            }
            Self::StringTooLong {
                offset,
                length,
                maximum,
            } => write!(
                formatter,
                "string at byte {offset} has {length} code units, maximum is {maximum}"
            ),
            Self::TrailingBytes { offset, remaining } => write!(
                formatter,
                "binary object has {remaining} trailing bytes at byte {offset}"
            ),
            Self::AllocationFailed => formatter.write_str("binary object allocation failed"),
        }
    }
}

impl std::error::Error for WireError {}

/// Exact QuickJS string payload representation.
///
/// Keeping width explicit is necessary for byte-for-byte cross-read: a wide
/// payload containing only Latin-1 code units is valid and must not be silently
/// compacted by this layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum WireString {
    Narrow(Box<[u8]>),
    Wide(Box<[u16]>),
}

impl WireString {
    #[must_use]
    pub(in crate::runtime) fn len(&self) -> usize {
        match self {
            Self::Narrow(bytes) => bytes.len(),
            Self::Wide(units) => units.len(),
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn is_wide(&self) -> bool {
        matches!(self, Self::Wide(_))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct BinaryObjectHeader {
    pub atom_count: u32,
}

/// Checked cursor over one immutable binary-object input.
pub(in crate::runtime) struct WireCursor<'a> {
    input: &'a [u8],
    offset: usize,
    mode: ReaderMode,
    limits: WireLimits,
    total_string_code_units: usize,
}

impl<'a> WireCursor<'a> {
    pub(in crate::runtime) fn new(
        input: &'a [u8],
        mode: ReaderMode,
        limits: WireLimits,
    ) -> Result<Self, WireError> {
        if input.len() > limits.max_input_bytes {
            return Err(WireError::ResourceLimit {
                kind: ResourceKind::InputBytes,
                requested: input.len(),
                limit: limits.max_input_bytes,
            });
        }
        Ok(Self {
            input,
            offset: 0,
            mode,
            limits,
            total_string_code_units: 0,
        })
    }

    #[must_use]
    pub(in crate::runtime) const fn position(&self) -> usize {
        self.offset
    }

    #[must_use]
    pub(in crate::runtime) const fn mode(&self) -> ReaderMode {
        self.mode
    }

    #[must_use]
    pub(in crate::runtime) fn remaining(&self) -> usize {
        self.input.len() - self.offset
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(WireError::LengthOverflow {
                offset: self.offset,
            })?;
        let Some(bytes) = self.input.get(self.offset..end) else {
            return Err(WireError::Truncated {
                offset: self.offset,
                needed: length,
                remaining: self.remaining(),
            });
        };
        self.offset = end;
        Ok(bytes)
    }

    pub(in crate::runtime) fn read_u8(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    pub(in crate::runtime) fn read_u16_le(&mut self) -> Result<u16, WireError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .expect("a two-byte checked slice must convert to an array");
        Ok(u16::from_le_bytes(bytes))
    }

    pub(in crate::runtime) fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], WireError> {
        self.take(length)
    }

    pub(in crate::runtime) fn read_tag(&mut self) -> Result<BcTag, WireError> {
        let tag = self.read_u8()?;
        // QuickJS reports the cursor position after consuming the bad tag.
        BcTag::from_byte(tag).ok_or(WireError::InvalidTag {
            tag,
            offset: self.offset,
        })
    }

    pub(in crate::runtime) fn read_uleb128(&mut self) -> Result<u32, WireError> {
        let start = self.offset;
        let mut value = 0_u32;
        let mut cursor = start;

        for index in 0..5_u32 {
            let Some(&byte) = self.input.get(cursor) else {
                return Err(WireError::Truncated {
                    offset: cursor,
                    needed: 1,
                    remaining: 0,
                });
            };
            cursor += 1;
            value |= u32::from(byte & 0x7f).wrapping_shl(index * 7);
            if byte & 0x80 == 0 {
                if self.mode == ReaderMode::Strict {
                    let (canonical, canonical_length) = encode_uleb128(value);
                    if self.input[start..cursor] != canonical[..canonical_length] {
                        return Err(WireError::NonCanonicalUleb128 { offset: start });
                    }
                }
                self.offset = cursor;
                return Ok(value);
            }
        }

        Err(WireError::MalformedUleb128 { offset: start })
    }

    pub(in crate::runtime) fn read_i32(&mut self) -> Result<i32, WireError> {
        let encoded = self.read_uleb128()?;
        Ok(decode_zigzag_i32(encoded))
    }

    pub(in crate::runtime) fn read_f64(&mut self) -> Result<f64, WireError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .expect("an eight-byte checked slice must convert to an array");
        Ok(f64::from_bits(u64::from_le_bytes(bytes)))
    }

    pub(in crate::runtime) fn read_header(&mut self) -> Result<BinaryObjectHeader, WireError> {
        let version = self.read_u8()?;
        if version != BC_VERSION {
            return Err(WireError::InvalidVersion {
                found: version,
                expected: BC_VERSION,
            });
        }
        let atom_count = self.read_uleb128()?;
        if atom_count > self.limits.max_atom_count {
            return Err(WireError::ResourceLimit {
                kind: ResourceKind::AtomCount,
                requested: atom_count as usize,
                limit: self.limits.max_atom_count as usize,
            });
        }
        Ok(BinaryObjectHeader { atom_count })
    }

    pub(in crate::runtime) fn read_string(&mut self) -> Result<WireString, WireError> {
        let length_offset = self.offset;
        let encoded_length = self.read_uleb128()?;
        let is_wide = encoded_length & 1 != 0;
        let length = (encoded_length >> 1) as usize;
        if length > MAX_STRING_CODE_UNITS {
            return Err(WireError::StringTooLong {
                offset: length_offset,
                length,
                maximum: MAX_STRING_CODE_UNITS,
            });
        }
        if length > self.limits.max_string_code_units {
            return Err(WireError::ResourceLimit {
                kind: ResourceKind::StringCodeUnits,
                requested: length,
                limit: self.limits.max_string_code_units,
            });
        }
        let total =
            self.total_string_code_units
                .checked_add(length)
                .ok_or(WireError::LengthOverflow {
                    offset: length_offset,
                })?;
        if total > self.limits.max_total_string_code_units {
            return Err(WireError::ResourceLimit {
                kind: ResourceKind::TotalStringCodeUnits,
                requested: total,
                limit: self.limits.max_total_string_code_units,
            });
        }

        let byte_length = if is_wide {
            length.checked_mul(2).ok_or(WireError::LengthOverflow {
                offset: length_offset,
            })?
        } else {
            length
        };
        let bytes = self.take(byte_length)?;
        self.total_string_code_units = total;

        if is_wide {
            let mut units = Vec::new();
            units
                .try_reserve_exact(length)
                .map_err(|_| WireError::AllocationFailed)?;
            let (pairs, remainder) = bytes.as_chunks::<2>();
            debug_assert!(remainder.is_empty());
            for pair in pairs {
                units.push(u16::from_le_bytes(*pair));
            }
            Ok(WireString::Wide(units.into_boxed_slice()))
        } else {
            let mut output = Vec::new();
            output
                .try_reserve_exact(length)
                .map_err(|_| WireError::AllocationFailed)?;
            output.extend_from_slice(bytes);
            Ok(WireString::Narrow(output.into_boxed_slice()))
        }
    }

    pub(in crate::runtime) fn finish(self) -> Result<(), WireError> {
        let remaining = self.remaining();
        if self.mode == ReaderMode::Strict && remaining != 0 {
            return Err(WireError::TrailingBytes {
                offset: self.offset,
                remaining,
            });
        }
        Ok(())
    }
}

/// Bounded canonical writer for the pure BCv5 primitives.
pub(in crate::runtime) struct WireWriter {
    output: Vec<u8>,
    max_output_bytes: usize,
}

impl WireWriter {
    #[must_use]
    pub(in crate::runtime) const fn new(max_output_bytes: usize) -> Self {
        Self {
            output: Vec::new(),
            max_output_bytes,
        }
    }

    fn reserve(&mut self, additional: usize) -> Result<(), WireError> {
        let requested =
            self.output
                .len()
                .checked_add(additional)
                .ok_or(WireError::LengthOverflow {
                    offset: self.output.len(),
                })?;
        if requested > self.max_output_bytes {
            return Err(WireError::ResourceLimit {
                kind: ResourceKind::OutputBytes,
                requested,
                limit: self.max_output_bytes,
            });
        }
        // Keep the logical byte limit exact, but let `Vec` grow geometrically.
        // Reserving the exact delta for every tag/ULEB append turns large
        // containers into a realloc-and-copy amplification path.
        self.output
            .try_reserve(additional)
            .map_err(|_| WireError::AllocationFailed)
    }

    pub(in crate::runtime) fn write_u8(&mut self, value: u8) -> Result<(), WireError> {
        self.reserve(1)?;
        self.output.push(value);
        Ok(())
    }

    pub(in crate::runtime) fn write_u16_le(&mut self, value: u16) -> Result<(), WireError> {
        self.write_bytes(&value.to_le_bytes())
    }

    pub(in crate::runtime) fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        self.reserve(bytes.len())?;
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    pub(in crate::runtime) fn write_tag(&mut self, tag: BcTag) -> Result<(), WireError> {
        self.write_u8(tag.to_byte())
    }

    pub(in crate::runtime) fn write_uleb128(&mut self, value: u32) -> Result<(), WireError> {
        let (encoded, length) = encode_uleb128(value);
        self.reserve(length)?;
        self.output.extend_from_slice(&encoded[..length]);
        Ok(())
    }

    pub(in crate::runtime) fn write_i32(&mut self, value: i32) -> Result<(), WireError> {
        self.write_uleb128(encode_zigzag_i32(value))
    }

    pub(in crate::runtime) fn write_f64(&mut self, value: f64) -> Result<(), WireError> {
        let bytes = value.to_bits().to_le_bytes();
        self.reserve(bytes.len())?;
        self.output.extend_from_slice(&bytes);
        Ok(())
    }

    pub(in crate::runtime) fn write_header(&mut self, atom_count: u32) -> Result<(), WireError> {
        let (encoded_count, count_length) = encode_uleb128(atom_count);
        self.reserve(1 + count_length)?;
        self.output.push(BC_VERSION);
        self.output
            .extend_from_slice(&encoded_count[..count_length]);
        Ok(())
    }

    pub(in crate::runtime) fn write_string(&mut self, value: &WireString) -> Result<(), WireError> {
        let length = value.len();
        if length > MAX_STRING_CODE_UNITS {
            return Err(WireError::StringTooLong {
                offset: self.output.len(),
                length,
                maximum: MAX_STRING_CODE_UNITS,
            });
        }
        let length = u32::try_from(length).map_err(|_| WireError::LengthOverflow {
            offset: self.output.len(),
        })?;
        let encoded_length = (length << 1) | u32::from(value.is_wide());
        let (length_bytes, length_byte_count) = encode_uleb128(encoded_length);
        let payload_bytes = match value {
            WireString::Narrow(bytes) => bytes.len(),
            WireString::Wide(units) => {
                units
                    .len()
                    .checked_mul(2)
                    .ok_or(WireError::LengthOverflow {
                        offset: self.output.len(),
                    })?
            }
        };
        let total_bytes =
            length_byte_count
                .checked_add(payload_bytes)
                .ok_or(WireError::LengthOverflow {
                    offset: self.output.len(),
                })?;
        self.reserve(total_bytes)?;
        self.output
            .extend_from_slice(&length_bytes[..length_byte_count]);
        match value {
            WireString::Narrow(bytes) => self.output.extend_from_slice(bytes),
            WireString::Wide(units) => {
                for unit in units {
                    self.output.extend_from_slice(&unit.to_le_bytes());
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub(in crate::runtime) fn as_bytes(&self) -> &[u8] {
        &self.output
    }

    #[must_use]
    pub(in crate::runtime) fn into_bytes(self) -> Vec<u8> {
        self.output
    }
}

const fn encode_zigzag_i32(value: i32) -> u32 {
    (value as u32).wrapping_shl(1) ^ ((value >> 31) as u32)
}

const fn decode_zigzag_i32(value: u32) -> i32 {
    ((value >> 1) as i32) ^ -((value & 1) as i32)
}

fn encode_uleb128(mut value: u32) -> ([u8; 5], usize) {
    let mut output = [0_u8; 5];
    let mut length = 0;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output[length] = byte;
        length += 1;
        if value == 0 {
            return (output, length);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_LIMITS: WireLimits = WireLimits::new(1024, 256, 128, 256);

    fn strict_cursor(input: &[u8]) -> WireCursor<'_> {
        WireCursor::new(input, ReaderMode::Strict, TEST_LIMITS).unwrap()
    }

    #[test]
    fn tags_match_the_pinned_quickjs_enum_exactly() {
        let expected = [
            BcTag::Null,
            BcTag::Undefined,
            BcTag::BoolFalse,
            BcTag::BoolTrue,
            BcTag::Int32,
            BcTag::Float64,
            BcTag::String,
            BcTag::Object,
            BcTag::Array,
            BcTag::BigInt,
            BcTag::TemplateObject,
            BcTag::FunctionBytecode,
            BcTag::Module,
            BcTag::TypedArray,
            BcTag::ArrayBuffer,
            BcTag::SharedArrayBuffer,
            BcTag::Date,
            BcTag::ObjectValue,
            BcTag::ObjectReference,
        ];
        for (index, expected_tag) in expected.into_iter().enumerate() {
            let byte = u8::try_from(index + 1).unwrap();
            assert_eq!(BcTag::from_byte(byte), Some(expected_tag));
            assert_eq!(expected_tag.to_byte(), byte);
        }
        assert_eq!(BcTag::from_byte(0), None);
        assert_eq!(BcTag::from_byte(20), None);

        let mut cursor = strict_cursor(&[0]);
        assert_eq!(
            cursor.read_tag(),
            Err(WireError::InvalidTag { tag: 0, offset: 1 })
        );
    }

    #[test]
    fn writer_emits_canonical_uleb128_and_strict_reader_accepts_it() {
        let cases: &[(u32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (16_383, &[0xff, 0x7f]),
            (u32::MAX, &[0xff, 0xff, 0xff, 0xff, 0x0f]),
        ];

        for &(value, bytes) in cases {
            let mut writer = WireWriter::new(5);
            writer.write_uleb128(value).unwrap();
            assert_eq!(writer.as_bytes(), bytes);

            let mut cursor = strict_cursor(bytes);
            assert_eq!(cursor.read_uleb128(), Ok(value));
            cursor.finish().unwrap();
        }
    }

    #[test]
    fn little_endian_u16_primitives_match_bc5_and_keep_exact_offsets() {
        let mut writer = WireWriter::new(4);
        writer.write_u16_le(0x1234).unwrap();
        writer.write_u16_le(u16::MAX).unwrap();
        assert_eq!(writer.as_bytes(), [0x34, 0x12, 0xff, 0xff]);

        let mut cursor = strict_cursor(writer.as_bytes());
        assert_eq!(cursor.read_u16_le(), Ok(0x1234));
        assert_eq!(cursor.position(), 2);
        assert_eq!(cursor.read_u16_le(), Ok(u16::MAX));
        cursor.finish().unwrap();

        let mut truncated = strict_cursor(&[0x34]);
        assert_eq!(
            truncated.read_u16_le(),
            Err(WireError::Truncated {
                offset: 0,
                needed: 2,
                remaining: 1,
            })
        );
    }

    #[test]
    fn reader_modes_separate_canonicality_and_quickjs_acceptance() {
        let non_minimal_zero = [0x80, 0x00];
        assert_eq!(
            strict_cursor(&non_minimal_zero).read_uleb128(),
            Err(WireError::NonCanonicalUleb128 { offset: 0 })
        );

        let mut compatible = WireCursor::new(
            &non_minimal_zero,
            ReaderMode::QuickJsCompatible,
            TEST_LIMITS,
        )
        .unwrap();
        assert_eq!(compatible.read_uleb128(), Ok(0));
        compatible.finish().unwrap();

        // Pinned QuickJS accepts all seven payload bits in the fifth byte;
        // unsigned truncation makes this another spelling of u32::MAX.
        let overflow_bits = [0xff, 0xff, 0xff, 0xff, 0x7f];
        assert_eq!(
            strict_cursor(&overflow_bits).read_uleb128(),
            Err(WireError::NonCanonicalUleb128 { offset: 0 })
        );
        let mut compatible =
            WireCursor::new(&overflow_bits, ReaderMode::QuickJsCompatible, TEST_LIMITS).unwrap();
        assert_eq!(compatible.read_uleb128(), Ok(u32::MAX));
    }

    #[test]
    fn malformed_and_truncated_uleb128_do_not_advance_the_cursor() {
        let mut truncated = strict_cursor(&[0x80]);
        assert!(matches!(
            truncated.read_uleb128(),
            Err(WireError::Truncated { offset: 1, .. })
        ));
        assert_eq!(truncated.position(), 0);

        let mut malformed = strict_cursor(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00]);
        assert_eq!(
            malformed.read_uleb128(),
            Err(WireError::MalformedUleb128 { offset: 0 })
        );
        assert_eq!(malformed.position(), 0);
    }

    #[test]
    fn zigzag_int32_matches_quickjs_edge_vectors() {
        let cases = [
            (0, 0_u32),
            (-1, 1),
            (1, 2),
            (i32::MAX, u32::MAX - 1),
            (i32::MIN, u32::MAX),
        ];
        for (value, encoded) in cases {
            assert_eq!(encode_zigzag_i32(value), encoded);
            assert_eq!(decode_zigzag_i32(encoded), value);

            let mut writer = WireWriter::new(5);
            writer.write_i32(value).unwrap();
            let bytes = writer.into_bytes();
            let mut cursor = strict_cursor(&bytes);
            assert_eq!(cursor.read_i32(), Ok(value));
        }
    }

    #[test]
    fn float64_is_exact_little_endian_bits() {
        let values = [
            1.5_f64,
            -0.0,
            f64::INFINITY,
            f64::from_bits(0x7ff8_0000_0000_0042),
        ];
        for value in values {
            let mut writer = WireWriter::new(8);
            writer.write_f64(value).unwrap();
            assert_eq!(writer.as_bytes(), &value.to_bits().to_le_bytes());

            let bytes = writer.into_bytes();
            let mut cursor = strict_cursor(&bytes);
            assert_eq!(cursor.read_f64().unwrap().to_bits(), value.to_bits());
        }
    }

    #[test]
    fn header_frames_version_and_atom_count() {
        let mut writer = WireWriter::new(8);
        writer.write_header(128).unwrap();
        assert_eq!(writer.as_bytes(), &[BC_VERSION, 0x80, 0x01]);

        let bytes = writer.into_bytes();
        let mut cursor = strict_cursor(&bytes);
        assert_eq!(
            cursor.read_header(),
            Ok(BinaryObjectHeader { atom_count: 128 })
        );
        cursor.finish().unwrap();

        let mut wrong_version = strict_cursor(&[4, 0]);
        assert_eq!(
            wrong_version.read_header(),
            Err(WireError::InvalidVersion {
                found: 4,
                expected: BC_VERSION,
            })
        );
    }

    #[test]
    fn atom_table_framing_composes_header_and_exact_string_payloads() {
        let atoms = [
            WireString::Narrow(Box::from(*b"x")),
            WireString::Wide(Box::from([0x20ac])),
        ];
        let mut writer = WireWriter::new(32);
        writer
            .write_header(u32::try_from(atoms.len()).unwrap())
            .unwrap();
        for atom in &atoms {
            writer.write_string(atom).unwrap();
        }
        assert_eq!(writer.as_bytes(), &[BC_VERSION, 2, 2, b'x', 3, 0xac, 0x20]);

        let bytes = writer.into_bytes();
        let mut cursor = strict_cursor(&bytes);
        let header = cursor.read_header().unwrap();
        assert_eq!(header.atom_count, 2);
        for expected in atoms {
            assert_eq!(cursor.read_string(), Ok(expected));
        }
        cursor.finish().unwrap();
    }

    #[test]
    fn atom_count_is_budgeted_at_the_header_boundary() {
        let limits = WireLimits::new(32, 3, 16, 16);
        let mut cursor = WireCursor::new(&[BC_VERSION, 4], ReaderMode::Strict, limits).unwrap();
        assert_eq!(
            cursor.read_header(),
            Err(WireError::ResourceLimit {
                kind: ResourceKind::AtomCount,
                requested: 4,
                limit: 3,
            })
        );
    }

    #[test]
    fn narrow_and_wide_strings_preserve_exact_wire_width() {
        let narrow = WireString::Narrow(Box::from([b'A', 0xe9]));
        let wide = WireString::Wide(Box::from([0x0041, 0x20ac, 0xd800]));
        let wide_latin1 = WireString::Wide(Box::from([0x0041, 0x00e9]));

        let mut writer = WireWriter::new(64);
        writer.write_string(&narrow).unwrap();
        writer.write_string(&wide).unwrap();
        writer.write_string(&wide_latin1).unwrap();
        assert_eq!(
            writer.as_bytes(),
            &[
                0x04, b'A', 0xe9, // narrow length 2
                0x07, 0x41, 0x00, 0xac, 0x20, 0x00, 0xd8, // wide length 3
                0x05, 0x41, 0x00, 0xe9, 0x00, // deliberately wide Latin-1
            ]
        );

        let bytes = writer.into_bytes();
        let mut cursor = strict_cursor(&bytes);
        assert_eq!(cursor.read_string(), Ok(narrow));
        assert_eq!(cursor.read_string(), Ok(wide));
        assert_eq!(cursor.read_string(), Ok(wide_latin1));
        cursor.finish().unwrap();
    }

    #[test]
    fn string_and_input_limits_fail_before_unbounded_allocation() {
        let input_limits = WireLimits::new(1, 1, 1, 1);
        assert!(matches!(
            WireCursor::new(&[0, 1], ReaderMode::Strict, input_limits),
            Err(WireError::ResourceLimit {
                kind: ResourceKind::InputBytes,
                ..
            })
        ));

        let string_limits = WireLimits::new(32, 1, 2, 3);
        let bytes = [0x06, b'a', b'b', b'c'];
        let mut cursor = WireCursor::new(&bytes, ReaderMode::Strict, string_limits).unwrap();
        assert_eq!(
            cursor.read_string(),
            Err(WireError::ResourceLimit {
                kind: ResourceKind::StringCodeUnits,
                requested: 3,
                limit: 2,
            })
        );

        let total_limits = WireLimits::new(32, 1, 2, 2);
        let bytes = [0x04, b'a', b'b', 0x02, b'c'];
        let mut cursor = WireCursor::new(&bytes, ReaderMode::Strict, total_limits).unwrap();
        assert!(cursor.read_string().is_ok());
        assert_eq!(
            cursor.read_string(),
            Err(WireError::ResourceLimit {
                kind: ResourceKind::TotalStringCodeUnits,
                requested: 3,
                limit: 2,
            })
        );

        let permissive = WireLimits::new(32, u32::MAX, usize::MAX, usize::MAX);
        let too_long = [0x80, 0x80, 0x80, 0x80, 0x08];
        let mut cursor = WireCursor::new(&too_long, ReaderMode::Strict, permissive).unwrap();
        assert_eq!(
            cursor.read_string(),
            Err(WireError::StringTooLong {
                offset: 0,
                length: MAX_STRING_CODE_UNITS + 1,
                maximum: MAX_STRING_CODE_UNITS,
            })
        );
    }

    #[test]
    fn checked_cursor_reports_payload_truncation_and_length_overflow() {
        let mut truncated = strict_cursor(&[0x05, 0x41, 0x00]);
        assert_eq!(
            truncated.read_string(),
            Err(WireError::Truncated {
                offset: 1,
                needed: 4,
                remaining: 2,
            })
        );

        let mut overflow = strict_cursor(&[0]);
        assert_eq!(overflow.read_u8(), Ok(0));
        assert_eq!(
            overflow.take(usize::MAX),
            Err(WireError::LengthOverflow { offset: 1 })
        );
    }

    #[test]
    fn trailing_bytes_are_mode_dependent() {
        let bytes = [BcTag::Null.to_byte(), 0xaa];
        let mut strict = strict_cursor(&bytes);
        assert_eq!(strict.read_tag(), Ok(BcTag::Null));
        assert_eq!(
            strict.finish(),
            Err(WireError::TrailingBytes {
                offset: 1,
                remaining: 1,
            })
        );

        let mut compatible =
            WireCursor::new(&bytes, ReaderMode::QuickJsCompatible, TEST_LIMITS).unwrap();
        assert_eq!(compatible.read_tag(), Ok(BcTag::Null));
        compatible.finish().unwrap();
    }

    #[test]
    fn writer_limit_failure_does_not_partially_append_a_primitive() {
        let mut writer = WireWriter::new(1);
        writer.write_u8(0xaa).unwrap();
        assert_eq!(
            writer.write_header(0),
            Err(WireError::ResourceLimit {
                kind: ResourceKind::OutputBytes,
                requested: 3,
                limit: 1,
            })
        );
        assert_eq!(writer.as_bytes(), &[0xaa]);
    }
}

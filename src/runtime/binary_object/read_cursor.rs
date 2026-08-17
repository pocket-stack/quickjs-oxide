//! Shared checked-read surface for complete BC5 inputs.
//!
//! The plain wire cursor and the inseparable SharedArrayBuffer transport cursor
//! need the same ordinary primitives while reading atoms, function envelopes,
//! modules, and data values. This sealed trait deliberately excludes both a raw
//! `u64` primitive and every SAB capability hook: only the separately sealed
//! data cursor may authenticate a complete SAB record.

use super::graph::sab_transport::SabTransportCursor;
use super::wire::{BcTag, BinaryObjectHeader, ReaderMode, WireCursor, WireError, WireString};

mod sealed {
    use super::{SabTransportCursor, WireCursor};

    pub trait Sealed {}

    impl Sealed for WireCursor<'_> {}
    impl Sealed for SabTransportCursor<'_> {}
}

/// Ordinary checked reads shared by the two authorized complete-input cursors.
///
/// The private supertrait fixes the implementation set to [`WireCursor`] and
/// [`SabTransportCursor`]. This surface deliberately has no fixed-width token
/// primitive or SAB authentication hook: structural dispatch remains solely
/// responsible for entering the complete checked SAB-record operation. The
/// payload reads are ordinary bounded wire reads, not a source-code security
/// boundary against a malicious helper implementation.
#[allow(private_bounds)]
pub(in crate::runtime::binary_object) trait CheckedReadCursor<'input>:
    sealed::Sealed
{
    fn position(&self) -> usize;
    fn mode(&self) -> ReaderMode;
    fn read_u8(&mut self) -> Result<u8, WireError>;
    fn read_u16_le(&mut self) -> Result<u16, WireError>;
    fn read_bytes(&mut self, length: usize) -> Result<&'input [u8], WireError>;
    fn read_tag(&mut self) -> Result<BcTag, WireError>;
    fn read_uleb128(&mut self) -> Result<u32, WireError>;
    fn read_i32(&mut self) -> Result<i32, WireError>;
    fn read_f64(&mut self) -> Result<f64, WireError>;
    fn read_header(&mut self) -> Result<BinaryObjectHeader, WireError>;
    fn read_string(&mut self) -> Result<WireString, WireError>;
    fn validate_wire_end(&self) -> Result<(), WireError>;
}

impl<'input> CheckedReadCursor<'input> for WireCursor<'input> {
    fn position(&self) -> usize {
        WireCursor::position(self)
    }

    fn mode(&self) -> ReaderMode {
        WireCursor::mode(self)
    }

    fn read_u8(&mut self) -> Result<u8, WireError> {
        WireCursor::read_u8(self)
    }

    fn read_u16_le(&mut self) -> Result<u16, WireError> {
        WireCursor::read_u16_le(self)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'input [u8], WireError> {
        WireCursor::read_bytes(self, length)
    }

    fn read_tag(&mut self) -> Result<BcTag, WireError> {
        WireCursor::read_tag(self)
    }

    fn read_uleb128(&mut self) -> Result<u32, WireError> {
        WireCursor::read_uleb128(self)
    }

    fn read_i32(&mut self) -> Result<i32, WireError> {
        WireCursor::read_i32(self)
    }

    fn read_f64(&mut self) -> Result<f64, WireError> {
        WireCursor::read_f64(self)
    }

    fn read_header(&mut self) -> Result<BinaryObjectHeader, WireError> {
        WireCursor::read_header(self)
    }

    fn read_string(&mut self) -> Result<WireString, WireError> {
        WireCursor::read_string(self)
    }

    fn validate_wire_end(&self) -> Result<(), WireError> {
        WireCursor::validate_wire_end(self)
    }
}

impl<'input> CheckedReadCursor<'input> for SabTransportCursor<'input> {
    fn position(&self) -> usize {
        SabTransportCursor::position(self)
    }

    fn mode(&self) -> ReaderMode {
        SabTransportCursor::mode(self)
    }

    fn read_u8(&mut self) -> Result<u8, WireError> {
        SabTransportCursor::read_u8(self)
    }

    fn read_u16_le(&mut self) -> Result<u16, WireError> {
        SabTransportCursor::read_u16_le(self)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'input [u8], WireError> {
        SabTransportCursor::read_bytes(self, length)
    }

    fn read_tag(&mut self) -> Result<BcTag, WireError> {
        SabTransportCursor::read_tag(self)
    }

    fn read_uleb128(&mut self) -> Result<u32, WireError> {
        SabTransportCursor::read_uleb128(self)
    }

    fn read_i32(&mut self) -> Result<i32, WireError> {
        SabTransportCursor::read_i32(self)
    }

    fn read_f64(&mut self) -> Result<f64, WireError> {
        SabTransportCursor::read_f64(self)
    }

    fn read_header(&mut self) -> Result<BinaryObjectHeader, WireError> {
        SabTransportCursor::read_header(self)
    }

    fn read_string(&mut self) -> Result<WireString, WireError> {
        SabTransportCursor::read_string(self)
    }

    fn validate_wire_end(&self) -> Result<(), WireError> {
        SabTransportCursor::validate_wire_end(self)
    }
}

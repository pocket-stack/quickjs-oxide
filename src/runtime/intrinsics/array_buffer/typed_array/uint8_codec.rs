//! Concrete `Uint8Array` base64 and hexadecimal codecs.
//!
//! QuickJS 2026-06-04 implements the proposal directly over the Uint8Array
//! backing range. The Rust port intentionally preserves its byte-oriented
//! WTF-8 input, option getter order, capacity short-circuits, and partial
//! writes on later syntax errors.

use super::*;

const BASE64_WHITESPACE: u8 = 64;
const BASE64_ERROR: u8 = 65;

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base64Alphabet {
    Base64,
    Base64Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LastChunkHandling {
    Loose,
    Strict,
    StopBeforePartial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DecodeProgress {
    read: usize,
    written: usize,
    invalid: bool,
}

impl Runtime {
    pub(super) fn call_uint8_array_codec(
        &self,
        realm: ContextId,
        kind: Uint8ArrayCodecKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            Uint8ArrayCodecKind::FromBase64 => {
                self.call_uint8_array_from_base64(realm, invocation, arguments)
            }
            Uint8ArrayCodecKind::FromHex => {
                self.call_uint8_array_from_hex(realm, invocation, arguments)
            }
            Uint8ArrayCodecKind::SetFromBase64 => {
                self.call_uint8_array_set_from_base64(realm, invocation, arguments)
            }
            Uint8ArrayCodecKind::SetFromHex => {
                self.call_uint8_array_set_from_hex(realm, invocation, arguments)
            }
            Uint8ArrayCodecKind::ToBase64 => {
                self.call_uint8_array_to_base64(realm, invocation, arguments)
            }
            Uint8ArrayCodecKind::ToHex => self.call_uint8_array_to_hex(realm, invocation),
        }
    }

    fn call_uint8_array_from_base64(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Uint8Array.fromBase64 received a constructor invocation",
            ));
        };
        let source = match self.uint8_codec_input_bytes(realm, arguments, 0)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let options = match self.uint8_codec_options(realm, arguments, 1)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let alphabet = match self.uint8_codec_alphabet(realm, options.as_ref())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let last_chunk = match self.uint8_codec_last_chunk(realm, options.as_ref())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let output_capacity = source
            .len()
            .checked_div(4)
            .and_then(|groups| groups.checked_mul(3))
            .and_then(|bytes| bytes.checked_add(3))
            .ok_or(RuntimeError::Invariant(
                "Uint8Array.fromBase64 capacity overflowed usize",
            ))?;
        let mut output = match self.uint8_codec_zeroed_bytes(realm, output_capacity)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let progress = decode_base64(&source, &mut output, alphabet, last_chunk);
        if progress.invalid {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Syntax,
                "invalid base64 string",
            )?));
        }
        output.truncate(progress.written);
        let result = match self.new_uint8_array_from_codec_bytes(realm, &output)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(result)))
    }

    fn call_uint8_array_from_hex(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Uint8Array.fromHex received a constructor invocation",
            ));
        };
        let source = match self.uint8_codec_input_bytes(realm, arguments, 0)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let output_capacity = source
            .len()
            .checked_div(2)
            .and_then(|bytes| bytes.checked_add(1))
            .ok_or(RuntimeError::Invariant(
                "Uint8Array.fromHex capacity overflowed usize",
            ))?;
        let mut output = match self.uint8_codec_zeroed_bytes(realm, output_capacity)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let progress = decode_hex(&source, &mut output);
        if progress.invalid {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Syntax,
                "invalid hex string",
            )?));
        }
        output.truncate(progress.written);
        let result = match self.new_uint8_array_from_codec_bytes(realm, &output)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(result)))
    }

    fn call_uint8_array_set_from_base64(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let target = match self.require_uint8_array_receiver(realm, invocation)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source = match self.uint8_codec_input_bytes(realm, arguments, 0)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let options = match self.uint8_codec_options(realm, arguments, 1)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let alphabet = match self.uint8_codec_alphabet(realm, options.as_ref())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let last_chunk = match self.uint8_codec_last_chunk(realm, options.as_ref())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state = match self.validated_uint8_codec_state(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start = typed_array_absolute_byte_offset(state.snapshot, 0)?;
        let length = usize::try_from(state.byte_length)
            .map_err(|_| RuntimeError::Invariant("Uint8Array byte length overflowed usize"))?;
        let access = self.snapshot_buffer_access(state.snapshot.buffer)?;
        let progress = self.with_buffer_range_mut(&access, start, length, |target| {
            decode_base64(&source, target, alphabet, last_chunk)
        })?;
        if progress.invalid {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Syntax,
                "invalid base64 string",
            )?));
        }
        self.make_uint8_codec_progress(realm, progress)
    }

    fn call_uint8_array_set_from_hex(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let target = match self.require_uint8_array_receiver(realm, invocation)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source = match self.uint8_codec_input_bytes(realm, arguments, 0)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state = match self.validated_uint8_codec_state(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start = typed_array_absolute_byte_offset(state.snapshot, 0)?;
        let length = usize::try_from(state.byte_length)
            .map_err(|_| RuntimeError::Invariant("Uint8Array byte length overflowed usize"))?;
        let access = self.snapshot_buffer_access(state.snapshot.buffer)?;
        let progress = self
            .with_buffer_range_mut(&access, start, length, |target| decode_hex(&source, target))?;
        if progress.invalid {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Syntax,
                "invalid hex string",
            )?));
        }
        self.make_uint8_codec_progress(realm, progress)
    }

    fn call_uint8_array_to_base64(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let target = match self.require_uint8_array_receiver(realm, invocation)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let options = match self.uint8_codec_options(realm, arguments, 0)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let alphabet = match self.uint8_codec_alphabet(realm, options.as_ref())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let omit_padding = match self.uint8_codec_omit_padding(realm, options.as_ref())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state = match self.validated_uint8_codec_state(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = usize::try_from(state.byte_length)
            .map_err(|_| RuntimeError::Invariant("Uint8Array byte length overflowed usize"))?;
        let output_length = length
            .checked_add(2)
            .and_then(|length| length.checked_div(3))
            .and_then(|groups| groups.checked_mul(4))
            .ok_or(RuntimeError::Invariant(
                "Uint8Array.toBase64 output length overflowed usize",
            ))?;
        if output_length > JsString::MAX_LEN {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "output too large",
            )?));
        }
        let mut output = match self.uint8_codec_zeroed_bytes(realm, output_length)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start = typed_array_absolute_byte_offset(state.snapshot, 0)?;
        let access = self.snapshot_buffer_access(state.snapshot.buffer)?;
        let written = self.with_buffer_range(&access, start, length, |source| {
            encode_base64(source, &mut output, alphabet)
        })?;
        debug_assert_eq!(written, output.len());
        if omit_padding {
            while output.last() == Some(&b'=') {
                output.truncate(output.len() - 1);
            }
        }
        Ok(Completion::Return(Value::String(
            JsString::from_owned_latin1(output),
        )))
    }

    fn call_uint8_array_to_hex(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let target = match self.require_uint8_array_receiver(realm, invocation)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state = match self.validated_uint8_codec_state(realm, &target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = usize::try_from(state.byte_length)
            .map_err(|_| RuntimeError::Invariant("Uint8Array byte length overflowed usize"))?;
        let output_length = length.checked_mul(2).ok_or(RuntimeError::Invariant(
            "Uint8Array.toHex output length overflowed usize",
        ))?;
        if output_length > JsString::MAX_LEN {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "output too large",
            )?));
        }
        let mut output = match self.uint8_codec_zeroed_bytes(realm, output_length)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start = typed_array_absolute_byte_offset(state.snapshot, 0)?;
        let access = self.snapshot_buffer_access(state.snapshot.buffer)?;
        self.with_buffer_range(&access, start, length, |source| {
            encode_hex(source, &mut output)
        })?;
        Ok(Completion::Return(Value::String(
            JsString::from_owned_latin1(output),
        )))
    }

    fn require_uint8_array_receiver(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Uint8Array codec received a constructor invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a Uint8Array",
            )?));
        };
        let Some(snapshot) = self.typed_array_snapshot_if_branded(&target)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a Uint8Array",
            )?));
        };
        if snapshot.element != TypedArrayElementKind::Uint8 {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a Uint8Array",
            )?));
        }
        Ok(NativeConversion::Value(target))
    }

    fn validated_uint8_codec_state(
        &self,
        realm: ContextId,
        target: &ObjectRef,
    ) -> Result<NativeConversion<TypedArrayState>, RuntimeError> {
        let state = self.typed_array_state(target)?;
        if state.out_of_bounds {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        }
        Ok(NativeConversion::Value(state))
    }

    fn uint8_codec_input_bytes(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
        index: usize,
    ) -> Result<NativeConversion<Vec<u8>>, RuntimeError> {
        let Some(Value::String(source)) = arguments.readable.get(index) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "expected string",
            )?));
        };
        match source.try_to_wtf8_bytes() {
            Ok(bytes) => Ok(NativeConversion::Value(bytes)),
            Err(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?)),
        }
    }

    fn uint8_codec_options(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
        index: usize,
    ) -> Result<NativeConversion<Option<ObjectRef>>, RuntimeError> {
        match arguments
            .readable
            .get(index)
            .ok_or(RuntimeError::Invariant(
                "Uint8Array codec options argv was not padded",
            ))? {
            Value::Undefined => Ok(NativeConversion::Value(None)),
            Value::Object(value) => Ok(NativeConversion::Value(Some(value.clone()))),
            Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "options must be an object",
            )?)),
        }
    }

    fn uint8_codec_alphabet(
        &self,
        realm: ContextId,
        options: Option<&ObjectRef>,
    ) -> Result<NativeConversion<Base64Alphabet>, RuntimeError> {
        let Some(options) = options else {
            return Ok(NativeConversion::Value(Base64Alphabet::Base64));
        };
        let key = self.intern_property_key("alphabet")?;
        let value = match self.get_property_in_realm(realm, options, &key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Value::String(value) = value else {
            if matches!(value, Value::Undefined) {
                return Ok(NativeConversion::Value(Base64Alphabet::Base64));
            }
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "expected string for alphabet",
            )?));
        };
        let bytes = match self.uint8_codec_c_string_bytes(realm, &value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        match bytes.as_slice() {
            b"base64" => Ok(NativeConversion::Value(Base64Alphabet::Base64)),
            b"base64url" => Ok(NativeConversion::Value(Base64Alphabet::Base64Url)),
            _ => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "invalid alphabet",
            )?)),
        }
    }

    fn uint8_codec_last_chunk(
        &self,
        realm: ContextId,
        options: Option<&ObjectRef>,
    ) -> Result<NativeConversion<LastChunkHandling>, RuntimeError> {
        let Some(options) = options else {
            return Ok(NativeConversion::Value(LastChunkHandling::Loose));
        };
        let key = self.intern_property_key("lastChunkHandling")?;
        let value = match self.get_property_in_realm(realm, options, &key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Value::String(value) = value else {
            if matches!(value, Value::Undefined) {
                return Ok(NativeConversion::Value(LastChunkHandling::Loose));
            }
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "expected string for lastChunkHandling",
            )?));
        };
        let bytes = match self.uint8_codec_c_string_bytes(realm, &value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        match bytes.as_slice() {
            b"loose" => Ok(NativeConversion::Value(LastChunkHandling::Loose)),
            b"strict" => Ok(NativeConversion::Value(LastChunkHandling::Strict)),
            b"stop-before-partial" => Ok(NativeConversion::Value(
                LastChunkHandling::StopBeforePartial,
            )),
            _ => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "invalid lastChunkHandling option",
            )?)),
        }
    }

    fn uint8_codec_omit_padding(
        &self,
        realm: ContextId,
        options: Option<&ObjectRef>,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Some(options) = options else {
            return Ok(NativeConversion::Value(false));
        };
        let key = self.intern_property_key("omitPadding")?;
        match self.get_property_in_realm(realm, options, &key)? {
            Completion::Return(value) => {
                Ok(NativeConversion::Value(self.value_to_boolean(&value)?))
            }
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn uint8_codec_c_string_bytes(
        &self,
        realm: ContextId,
        value: &JsString,
    ) -> Result<NativeConversion<Vec<u8>>, RuntimeError> {
        let mut bytes = match value.try_to_wtf8_bytes() {
            Ok(value) => value,
            Err(_) => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
        };
        if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
            bytes.truncate(nul);
        }
        Ok(NativeConversion::Value(bytes))
    }

    fn uint8_codec_zeroed_bytes(
        &self,
        realm: ContextId,
        length: usize,
    ) -> Result<NativeConversion<Vec<u8>>, RuntimeError> {
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(length).is_err() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "out of memory",
            )?));
        }
        bytes.resize(length, 0);
        Ok(NativeConversion::Value(bytes))
    }

    fn new_uint8_array_from_codec_bytes(
        &self,
        realm: ContextId,
        bytes: &[u8],
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let prototype = self.typed_array_default_prototype(realm, TypedArrayElementKind::Uint8)?;
        let length = u64::try_from(bytes.len()).map_err(|_| {
            RuntimeError::Invariant("Uint8Array codec output length overflowed u64")
        })?;
        let target = match self.new_typed_array_for_length(
            realm,
            &prototype,
            TypedArrayElementKind::Uint8,
            length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let state = self.typed_array_state(&target)?;
        let start = typed_array_absolute_byte_offset(state.snapshot, 0)?;
        let access = self.snapshot_buffer_access(state.snapshot.buffer)?;
        self.with_buffer_range_mut(&access, start, bytes.len(), |target| {
            target.copy_from_slice(bytes)
        })?;
        Ok(NativeConversion::Value(target))
    }

    fn make_uint8_codec_progress(
        &self,
        realm: ContextId,
        progress: DecodeProgress,
    ) -> Result<Completion, RuntimeError> {
        let object_prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
        let object_prototype = ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?;
        let result = self.new_object(Some(&object_prototype))?;
        for (name, value) in [("read", progress.read), ("written", progress.written)] {
            let value = u32::try_from(value)
                .map(typed_array_u32_value)
                .map_err(|_| RuntimeError::Invariant("Uint8Array codec count overflowed u32"))?;
            let key = self.intern_property_key(name)?;
            if !self.define_own_property(
                &result,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                return Err(RuntimeError::Invariant(
                    "Uint8Array codec result property definition was rejected",
                ));
            }
        }
        Ok(Completion::Return(Value::Object(result)))
    }
}

fn encode_base64(source: &[u8], target: &mut [u8], alphabet: Base64Alphabet) -> usize {
    let alphabet = match alphabet {
        Base64Alphabet::Base64 => BASE64_ALPHABET,
        Base64Alphabet::Base64Url => BASE64URL_ALPHABET,
    };
    let mut source_index = 0;
    let mut target_index = 0;
    while source_index + 3 <= source.len() {
        let value = (u32::from(source[source_index]) << 16)
            | (u32::from(source[source_index + 1]) << 8)
            | u32::from(source[source_index + 2]);
        target[target_index] = alphabet[((value >> 18) & 63) as usize];
        target[target_index + 1] = alphabet[((value >> 12) & 63) as usize];
        target[target_index + 2] = alphabet[((value >> 6) & 63) as usize];
        target[target_index + 3] = alphabet[(value & 63) as usize];
        source_index += 3;
        target_index += 4;
    }
    match source.len() - source_index {
        1 => {
            let value = u32::from(source[source_index]) << 16;
            target[target_index] = alphabet[((value >> 18) & 63) as usize];
            target[target_index + 1] = alphabet[((value >> 12) & 63) as usize];
            target[target_index + 2] = b'=';
            target[target_index + 3] = b'=';
            target_index += 4;
        }
        2 => {
            let value = (u32::from(source[source_index]) << 16)
                | (u32::from(source[source_index + 1]) << 8);
            target[target_index] = alphabet[((value >> 18) & 63) as usize];
            target[target_index + 1] = alphabet[((value >> 12) & 63) as usize];
            target[target_index + 2] = alphabet[((value >> 6) & 63) as usize];
            target[target_index + 3] = b'=';
            target_index += 4;
        }
        0 => {}
        _ => unreachable!("base64 remainder exceeded two bytes"),
    }
    target_index
}

fn encode_hex(source: &[u8], target: &mut [u8]) {
    debug_assert_eq!(target.len(), source.len() * 2);
    for (index, byte) in source.iter().copied().enumerate() {
        target[index * 2] = HEX_DIGITS[usize::from(byte >> 4)];
        target[index * 2 + 1] = HEX_DIGITS[usize::from(byte & 0x0f)];
    }
}

fn decode_base64(
    source: &[u8],
    target: &mut [u8],
    alphabet: Base64Alphabet,
    last_chunk: LastChunkHandling,
) -> DecodeProgress {
    if target.is_empty() {
        return DecodeProgress {
            read: 0,
            written: 0,
            invalid: false,
        };
    }

    let mut read = 0;
    let mut written = 0;
    let mut accumulator = 0_u32;
    let mut seen = 0_u8;
    let mut index = 0;

    loop {
        index = skip_base64_whitespace(source, index, alphabet);
        if index == source.len() {
            if seen > 0 {
                if last_chunk == LastChunkHandling::StopBeforePartial {
                    return DecodeProgress {
                        read,
                        written,
                        invalid: false,
                    };
                }
                if last_chunk == LastChunkHandling::Strict || seen == 1 {
                    return DecodeProgress {
                        read,
                        written,
                        invalid: true,
                    };
                }
                break;
            }
            return DecodeProgress {
                read: source.len(),
                written,
                invalid: false,
            };
        }

        let byte = source[index];
        index += 1;
        if byte == b'=' {
            if seen < 2 {
                return DecodeProgress {
                    read,
                    written,
                    invalid: true,
                };
            }
            index = skip_base64_whitespace(source, index, alphabet);
            if seen == 2 {
                if index == source.len() {
                    if last_chunk == LastChunkHandling::StopBeforePartial {
                        return DecodeProgress {
                            read,
                            written,
                            invalid: false,
                        };
                    }
                    return DecodeProgress {
                        read,
                        written,
                        invalid: true,
                    };
                }
                if source[index] != b'=' {
                    return DecodeProgress {
                        read,
                        written,
                        invalid: true,
                    };
                }
                index += 1;
                index = skip_base64_whitespace(source, index, alphabet);
            }
            if index != source.len() {
                return DecodeProgress {
                    read,
                    written,
                    invalid: true,
                };
            }
            if last_chunk == LastChunkHandling::Strict {
                let mask = if seen == 2 { 0x0f } else { 0x03 };
                if accumulator & mask != 0 {
                    return DecodeProgress {
                        read,
                        written,
                        invalid: true,
                    };
                }
            }
            break;
        }

        let value = decode_base64_byte(byte, alphabet);
        if value >= 64 {
            return DecodeProgress {
                read,
                written,
                invalid: true,
            };
        }

        let remaining = target.len() - written;
        if (remaining == 1 && seen == 2) || (remaining == 2 && seen == 3) {
            return DecodeProgress {
                read,
                written,
                invalid: false,
            };
        }

        accumulator = (accumulator << 6) | u32::from(value);
        seen += 1;
        if seen == 4 {
            target[written] = (accumulator >> 16) as u8;
            target[written + 1] = (accumulator >> 8) as u8;
            target[written + 2] = accumulator as u8;
            written += 3;
            accumulator = 0;
            seen = 0;
            read = index;
            if written >= target.len() {
                return DecodeProgress {
                    read,
                    written,
                    invalid: false,
                };
            }
        }
    }

    if seen == 2 {
        target[written] = (accumulator >> 4) as u8;
        written += 1;
    } else if seen == 3 {
        target[written] = (accumulator >> 10) as u8;
        target[written + 1] = (accumulator >> 2) as u8;
        written += 2;
    }
    DecodeProgress {
        read: source.len(),
        written,
        invalid: false,
    }
}

fn decode_hex(source: &[u8], target: &mut [u8]) -> DecodeProgress {
    if source.len() % 2 != 0 {
        return DecodeProgress {
            read: 0,
            written: 0,
            invalid: true,
        };
    }
    let mut read = 0;
    let mut written = 0;
    while read < source.len() && written < target.len() {
        let Some(high) = decode_hex_digit(source[read]) else {
            return DecodeProgress {
                read,
                written,
                invalid: true,
            };
        };
        let Some(low) = decode_hex_digit(source[read + 1]) else {
            return DecodeProgress {
                read,
                written,
                invalid: true,
            };
        };
        target[written] = (high << 4) | low;
        written += 1;
        read += 2;
    }
    DecodeProgress {
        read,
        written,
        invalid: false,
    }
}

fn skip_base64_whitespace(source: &[u8], mut index: usize, alphabet: Base64Alphabet) -> usize {
    while index < source.len() && decode_base64_byte(source[index], alphabet) == BASE64_WHITESPACE {
        index += 1;
    }
    index
}

fn decode_base64_byte(byte: u8, alphabet: Base64Alphabet) -> u8 {
    match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' if alphabet == Base64Alphabet::Base64 => 62,
        b'/' if alphabet == Base64Alphabet::Base64 => 63,
        b'-' if alphabet == Base64Alphabet::Base64Url => 62,
        b'_' if alphabet == Base64Alphabet::Base64Url => 63,
        b'\t' | b'\n' | b'\x0c' | b'\r' | b' ' => BASE64_WHITESPACE,
        _ => BASE64_ERROR,
    }
}

fn decode_hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_kernel_preserves_quickjs_capacity_and_partial_rules() {
        let mut zero = [];
        assert_eq!(
            decode_base64(
                b"###",
                &mut zero,
                Base64Alphabet::Base64,
                LastChunkHandling::Strict,
            ),
            DecodeProgress {
                read: 0,
                written: 0,
                invalid: false,
            }
        );

        let mut one = [0xff];
        assert_eq!(
            decode_base64(
                b"YWI=",
                &mut one,
                Base64Alphabet::Base64,
                LastChunkHandling::Loose,
            ),
            DecodeProgress {
                read: 0,
                written: 0,
                invalid: false,
            }
        );
        assert_eq!(one, [0xff]);

        let mut prefix = [0xff; 6];
        let progress = decode_base64(
            b"AAAA.AAA",
            &mut prefix,
            Base64Alphabet::Base64,
            LastChunkHandling::Loose,
        );
        assert!(progress.invalid);
        assert_eq!(&prefix[..3], &[0, 0, 0]);
        assert_eq!(&prefix[3..], &[0xff; 3]);
    }

    #[test]
    fn base64_kernel_distinguishes_last_chunk_modes_and_alphabets() {
        let mut output = [0; 3];
        assert_eq!(
            decode_base64(
                b"YQ",
                &mut output,
                Base64Alphabet::Base64,
                LastChunkHandling::Loose,
            ),
            DecodeProgress {
                read: 2,
                written: 1,
                invalid: false,
            }
        );
        assert_eq!(output[0], b'a');

        assert!(
            decode_base64(
                b"YQ",
                &mut output,
                Base64Alphabet::Base64,
                LastChunkHandling::Strict,
            )
            .invalid
        );
        assert_eq!(
            decode_base64(
                b"YQ",
                &mut output,
                Base64Alphabet::Base64,
                LastChunkHandling::StopBeforePartial,
            ),
            DecodeProgress {
                read: 0,
                written: 0,
                invalid: false,
            }
        );

        let mut url = [0; 3];
        assert!(
            !decode_base64(
                b"-_8=",
                &mut url,
                Base64Alphabet::Base64Url,
                LastChunkHandling::Loose,
            )
            .invalid
        );
        assert_eq!(&url[..2], &[0xfb, 0xff]);
        assert!(
            decode_base64(
                b"-_8=",
                &mut url,
                Base64Alphabet::Base64,
                LastChunkHandling::Loose,
            )
            .invalid
        );
    }

    #[test]
    fn hex_kernel_checks_odd_length_before_capacity_and_keeps_prefix() {
        let mut zero = [];
        assert!(decode_hex(b"1", &mut zero).invalid);
        assert_eq!(
            decode_hex(b"gg", &mut zero),
            DecodeProgress {
                read: 0,
                written: 0,
                invalid: false,
            }
        );

        let mut output = [0xff; 3];
        let progress = decode_hex(b"aaag", &mut output);
        assert!(progress.invalid);
        assert_eq!(output, [0xaa, 0xff, 0xff]);
    }

    #[test]
    fn encoders_emit_pinned_lowercase_hex_and_padding() {
        let mut base64 = [0; 8];
        assert_eq!(
            encode_base64(
                &[0xfb, 0xff, 0x00, 0x61],
                &mut base64,
                Base64Alphabet::Base64Url,
            ),
            8
        );
        assert_eq!(&base64, b"-_8AYQ==");

        let mut hex = [0; 6];
        encode_hex(&[0x00, 0xaf, 0xff], &mut hex);
        assert_eq!(&hex, b"00afff");
    }
}

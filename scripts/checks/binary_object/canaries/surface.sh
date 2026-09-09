expect_rejected vm-dependency forbidden-vm-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::vm::Completion;'
expect_rejected compiler-dependency forbidden-compiler-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::{atom::Atom, compiler as parser};'
expect_rejected heap-dependency forbidden-heap-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::heap as runtime_heap;'
expect_rejected grouped-heap-dependency forbidden-heap-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::{atom::Atom, heap as runtime_heap};'
expect_rejected runtime-dependency forbidden-heap-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::heap::runtime as engine_runtime;'
expect_rejected runtime-representation forbidden-runtime-representation \
    src/engine/code/binary_object/atoms.rs \
    'type Published = FunctionBytecodeData;'
expect_rejected codec-publication forbidden-publication-boundary \
    src/engine/code/binary_object/atoms.rs \
    'fn publish(runtime: Runtime) { runtime.publish_unlinked_function(realm, function); }'
expect_rejected shared-memory-dependency forbidden-heap-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::heap::shared_memory as runtime_shared_memory;'
expect_rejected parent-shared-memory-dependency forbidden-shared-memory-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use super::shared_memory as runtime_shared_memory;'
expect_rejected grouped-shared-memory-dependency forbidden-shared-memory-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::{atom::Atom, shared_memory as runtime_shared_memory};'
expect_rejected parent-grouped-shared-memory-dependency forbidden-shared-memory-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use super::{atoms, shared_memory as runtime_shared_memory};'
expect_rejected shared-buffer-handle forbidden-shared-memory-runtime-type \
    src/engine/code/binary_object/atoms.rs \
    'type RuntimeBacking = SharedBufferHandle;'
expect_rejected shared-backing-store forbidden-shared-memory-runtime-type \
    src/engine/code/binary_object/atoms.rs \
    'type RuntimeBacking = SharedBackingStore;'
expect_rejected unsafe-block forbidden-unsafe-code \
    src/engine/code/binary_object/atoms.rs \
    'fn bridge() { unsafe { core::hint::unreachable_unchecked() } }'
expect_rejected unsafe-function forbidden-unsafe-code \
    src/engine/code/binary_object/atoms.rs \
    'unsafe fn bridge() {}'
expect_rejected unsafe-impl forbidden-unsafe-code \
    src/engine/code/binary_object/atoms.rs \
    'unsafe impl Send for Archive {}'
expect_rejected unsafe-trait forbidden-unsafe-code \
    src/engine/code/binary_object/atoms.rs \
    'unsafe trait NativeArchive {}'
expect_rejected non-null-pointer forbidden-non-null-pointer \
    src/engine/code/binary_object/atoms.rs \
    'type NativeAddress = core::ptr::NonNull<u8>;'
expect_rejected raw-const-pointer forbidden-raw-pointer-type \
    src/engine/code/binary_object/atoms.rs \
    'type NativeAddress = *const u8;'
expect_rejected raw-mut-pointer forbidden-raw-pointer-type \
    src/engine/code/binary_object/atoms.rs \
    'type NativeAddress = *mut u8;'
expect_rejected from-raw-parts forbidden-native-pointer-bridge \
    src/engine/code/binary_object/atoms.rs \
    'let bytes = core::slice::from_raw_parts(address, length);'
expect_rejected from-raw-parts-mut forbidden-native-pointer-bridge \
    src/engine/code/binary_object/atoms.rs \
    'let bytes = core::slice::from_raw_parts_mut(address, length);'
expect_rejected into-raw forbidden-native-pointer-bridge \
    src/engine/code/binary_object/atoms.rs \
    'let address = Box::into_raw(value);'
expect_rejected qualified-from-raw forbidden-native-pointer-bridge \
    src/engine/code/binary_object/atoms.rs \
    'let value = Box::from_raw(address);'
expect_rejected bytecode-function forbidden-bytecode-function \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::code::bytecode::BytecodeFunction;'
expect_rejected function-bytecode-ref forbidden-bytecode-function \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::code::bytecode::FunctionBytecodeRef;'
expect_rejected public-lib public-lib-boundary \
    src/lib.rs \
    'pub use runtime::binary_object;'
expect_rejected lib-path-alias public-lib-boundary \
    src/lib.rs \
    '#[path = "runtime/binary_object/mod.rs"] pub mod archive;'
expect_rejected runtime-consumer runtime-boundary \
    src/engine/code/mod.rs \
    'use self::binary_object::bytecode_image::decode_bytecode_image;'
expect_rewrite_rejected consumer-public-module binary-object-consumer-module \
    src/engine/code/mod.rs \
    'mod binary_object_publish;' \
    'pub(super) mod binary_object_publish;'
expect_rejected second-binary-object-consumer binary-object-consumer-set \
    src/engine/heap/runtime/other.rs \
    'use super::binary_object::{ScalarValueDraft, decode_trusted_scalar_script};'
expect_rejected alternate-binary-object-path binary-object-consumer-set \
    src/engine/heap/runtime/other.rs \
    '#[path = "binary_object/mod.rs"] mod alternate_archive;'
expect_rejected second-scalar-facade-consumer binary-object-facade-consumer-set \
    src/engine/heap/runtime/other.rs \
    'fn leak() { let _ = decode_trusted_scalar_script(bytes); }'
expect_rejected consumer-codec-import binary-object-consumer-import \
    src/engine/code/binary_object_publish.rs \
    'use super::binary_object::BytecodeImage;'
expect_rejected consumer-atom-string binary-object-consumer-atom-string \
    src/engine/code/binary_object_publish.rs \
    'fn atom_constant(value: JsString) { let _ = UnlinkedConstant::atom_string(value); }'
expect_rejected consumer-atom-string-alias binary-object-consumer-atom-string \
    src/engine/code/binary_object_publish.rs \
    'type C = UnlinkedConstant; fn aliased_atom_constant(value: JsString) { let _ = C::atom_string(value); }'
expect_rejected consumer-atom-interning binary-object-consumer-atom-interning \
    src/engine/code/binary_object_publish.rs \
    'fn intern_directly(runtime: &Runtime) { let _ = runtime.intern_property_key("forbidden"); }'
expect_rejected consumer-second-publisher binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    'fn publish_twice(runtime: &Runtime) { let _ = runtime.publish_unlinked_function(realm, function); }'
expect_rejected consumer-verifier-bypass binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    'fn bypass(runtime: &Runtime) { runtime.publish_verified_unlinked_function(realm, function); }'
expect_rejected consumer-dead-safe-alternate-publication binary-object-consumer-alternate-entrypoint \
    src/engine/code/binary_object_publish.rs \
    'fn alternate(runtime: &Runtime) { if false { let _ = runtime.publish_unlinked_function(realm, function); } let _ = runtime.compile_in_realm(realm, source); }'
expect_rejected consumer-heap-type binary-object-consumer-heap-type \
    src/engine/code/binary_object_publish.rs \
    'fn allocate_directly(value: FunctionBytecodeData) {}'
expect_rejected consumer-root-forge binary-object-consumer-root-forge \
    src/engine/code/binary_object_publish.rs \
    'fn forge(runtime: Runtime, id: FunctionBytecodeId) { let _ = FunctionBytecodeRef::from_owned_handle(runtime, id); }'
expect_rewrite_rejected consumer-lowered-scalar-unary-vector binary-object-consumer-scalar-mapping \
    src/engine/code/binary_object_publish.rs \
    '    IntegerAtomString(u32),' \
    $'    IntegerAtomString(u32),\n    Unary(Vec<Instruction>),'
expect_rewrite_rejected consumer-float-normalization binary-object-consumer-float64 \
    src/engine/code/binary_object_publish.rs \
    'lower_primitive_constant(Value::Float(f64::from_bits(bits)))' \
    'lower_primitive_constant(Value::number(f64::from_bits(bits)))'
expect_rewrite_rejected consumer-pool-atom-swap binary-object-consumer-scalar-mapping \
    src/engine/code/binary_object_publish.rs \
    $'        ScalarValueDraft::ConstantString(value) => lower_scalar_string(value)\n            .and_then(|value| lower_primitive_constant(Value::String(value)))\n            .map(LoweredScalar::Constant),' \
    $'        ScalarValueDraft::ConstantString(value) => Ok(LoweredScalar::AtomString(\n            UnlinkedConstant::atom_string(lower_scalar_string(value)?),\n        )),'
expect_rewrite_rejected consumer-integer-via-cpool binary-object-consumer-scalar-mapping \
    src/engine/code/binary_object_publish.rs \
    'ScalarValueDraft::IntegerAtomString(value) => Ok(LoweredScalar::IntegerAtomString(value)),' \
    'ScalarValueDraft::IntegerAtomString(value) => lower_primitive_constant(Value::String(JsString::from_fresh_decimal_u32(value))).map(LoweredScalar::Constant),'
expect_rewrite_rejected consumer-empty-primitive binary-object-consumer-scalar-mapping \
    src/engine/code/binary_object_publish.rs \
    $'        ScalarValueDraft::EmptyString => Ok(LoweredScalar::AtomString(\n            UnlinkedConstant::atom_string(JsString::from_static("")),\n        )),' \
    $'        ScalarValueDraft::EmptyString => Ok(LoweredScalar::Constant(\n            lower_primitive_constant(Value::String(JsString::from_static("")))?,\n        )),'
expect_rewrite_rejected consumer-bigint-dead-path-coercion binary-object-consumer-scalar-mapping \
    src/engine/code/binary_object_publish.rs \
    $'        ScalarValueDraft::BigIntBytes(bytes) => {\n            lower_bigint_constant(&bytes).map(LoweredScalar::Constant)\n        }' \
    $'        ScalarValueDraft::BigIntBytes(bytes) => {\n            if false { return lower_bigint_constant(&bytes).map(LoweredScalar::Constant); }\n            Ok(LoweredScalar::Direct(Instruction::PushI32(i32::from(bytes.first().copied().unwrap_or(0)))))\n        }'
expect_rewrite_rejected consumer-bigint-noncanonical-decode binary-object-consumer-bigint \
    src/engine/code/binary_object_publish.rs \
    'JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)' \
    'JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), false)'
expect_rewrite_rejected consumer-bigint-partial-consumption binary-object-consumer-bigint \
    src/engine/code/binary_object_publish.rs \
    '    if consumed != bytes.len() {' \
    '    if false {'
expect_rewrite_rejected consumer-bigint-input-shadow binary-object-consumer-bigint \
    src/engine/code/binary_object_publish.rs \
    '    let (value, consumed) =' \
    $'    let bytes = &bytes[..1];\n    let (value, consumed) ='
expect_rewrite_rejected consumer-unary-negation-mapping-drift binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    '                ScalarUnaryOp::Neg => Instruction::Neg,' \
    '                ScalarUnaryOp::Neg => Instruction::Nop,'
expect_rewrite_rejected consumer-unary-chain-reorder binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    '        for operation in unary_ops {' \
    '        for operation in unary_ops.into_iter().rev() {'
expect_rewrite_rejected consumer-unary-eager-precompute binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    '        let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;' \
    $'        let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;\n        let value = match value { ScalarValueDraft::Int(value) => ScalarValueDraft::Int(value.wrapping_neg()), value => value };'
expect_rejected consumer-bigint-eager-negation binary-object-consumer-bigint-eager-negation \
    src/engine/code/binary_object_publish.rs \
    'fn eager_negation(value: JsBigInt) { let _ = std::ops::Neg::neg(value); }'
expect_rewrite_rejected consumer-skips-safe-publication binary-object-consumer-publication \
    src/engine/code/binary_object_publish.rs \
    '        self.publish_unlinked_function(realm, function)' \
    '        self.compile_in_realm(realm, source)'


expect_rejected nested-engine-vm-dependency forbidden-vm-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::vm as execution;'
expect_rejected nested-engine-heap-dependency forbidden-heap-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::heap as storage;'
expect_rejected nested-engine-compiler-dependency forbidden-compiler-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::compiler as parser;'
expect_rejected nested-engine-grouped-dependency forbidden-vm-dependency \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine::{vm as execution};'
expect_rejected nested-engine-root-alias forbidden-crate-alias \
    src/engine/code/binary_object/atoms.rs \
    'use crate::engine as implementation;'

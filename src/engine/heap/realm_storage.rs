use super::*;

impl Heap {
    /// Finish two-phase native-function bootstrap by installing its defining
    /// realm as an owned GC edge.
    ///
    /// Realm construction is necessarily cyclic: the Context owns
    /// `%Function.prototype%`, while that native callable owns its defining
    /// Context. The object is therefore allocated provisionally, the Context
    /// is published, and this operation closes the cycle transactionally.
    pub(crate) fn attach_native_function_realm(
        &mut self,
        object: ObjectId,
        realm: ContextId,
    ) -> Result<(), HeapError> {
        self.context(realm)?;
        match &self.object(object)?.payload {
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: None, .. },
                ..
            } => {}
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: Some(_), .. },
                ..
            } => {
                return Err(HeapError::Invariant(
                    "native function already has a defining realm",
                ));
            }
            ObjectPayload::Ordinary
            | ObjectPayload::RawJson
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::WeakMap { .. }
            | ObjectPayload::WeakSet { .. }
            | ObjectPayload::WeakRef { .. }
            | ObjectPayload::FinalizationRegistry(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::IteratorHelper(_)
            | ObjectPayload::IteratorWrap(_)
            | ObjectPayload::AsyncFromSyncIterator(_)
            | ObjectPayload::IteratorConcat(_)
            | ObjectPayload::Proxy(_)
            | ObjectPayload::ArrayBuffer(_)
            | ObjectPayload::SharedArrayBuffer(_)
            | ObjectPayload::DataView(_)
            | ObjectPayload::TypedArray(_)
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::Generator { .. }
            | ObjectPayload::AsyncGenerator(_)
            | ObjectPayload::AsyncFunctionState(_)
            | ObjectPayload::Promise(_) => {
                return Err(HeapError::Invariant(
                    "attempted to attach a native realm to a non-native function",
                ));
            }
        }

        self.retain_raw(RawId::Context(realm), 1)?;
        let ObjectPayload::NativeFunction { data, .. } = &mut self.object_mut(object)?.payload
        else {
            unreachable!("native-function payload was validated before retaining its realm")
        };
        data.realm = Some(realm);
        Ok(())
    }

    /// Publish the realm's shared frozen `%ThrowTypeError%` root after the
    /// context exists. Its native callable already owns the context, so this
    /// deliberately closes the same collectable realm cycle as QuickJS.
    pub(crate) fn attach_throw_type_error(
        &mut self,
        realm: ContextId,
        thrower: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.throw_type_error.is_some() {
            return Err(HeapError::Invariant(
                "context already has a %ThrowTypeError% root",
            ));
        }
        if !matches!(
            self.object(thrower)?.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: NativeFunctionId::ThrowTypeError,
                    realm: Some(target_realm),
                    ..
                },
                ..
            } if target_realm == realm
        ) {
            return Err(HeapError::Invariant(
                "%ThrowTypeError% root is not the realm's poison native function",
            ));
        }

        self.retain_raw(RawId::Object(thrower), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining the thrower")
        };
        context.throw_type_error = Some(thrower);
        Ok(())
    }

    /// Cache the original realm-local `Array.prototype.values` callable.
    /// This is a distinct Context root because the public prototype property
    /// is writable and configurable while arguments creation must keep using
    /// the bootstrap identity.
    pub(crate) fn attach_array_prototype_values(
        &mut self,
        realm: ContextId,
        values: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.array_prototype_values.is_some() {
            return Err(HeapError::Invariant(
                "context already has an Array.prototype.values cache root",
            ));
        }
        if !matches!(
            self.object(values)?.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: NativeFunctionId::ArrayPrototypeIterator(ArrayIteratorKind::Value),
                    realm: Some(target_realm),
                    ..
                },
                ..
            } if target_realm == realm
        ) {
            return Err(HeapError::Invariant(
                "Array.prototype.values cache is not the realm's values native",
            ));
        }

        self.retain_raw(RawId::Object(values), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Array values")
        };
        context.array_prototype_values = Some(values);
        Ok(())
    }

    /// Publish the realm's `%Function%` root after its native callable and
    /// constructor/prototype cycle have been fully initialized.
    pub(crate) fn attach_function_constructor(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.function_constructor.is_some() {
            return Err(HeapError::Invariant(
                "context already has a Function constructor root",
            ));
        }
        let constructor_object = self.object(constructor)?;
        if !constructor_object.is_constructor
            || !matches!(
                constructor_object.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Function constructor root is not the realm's Function native",
            ));
        }

        self.retain_raw(RawId::Object(constructor), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Function")
        };
        context.function_constructor = Some(constructor);
        Ok(())
    }

    /// Cache the realm's original `%eval%` callable independently from its
    /// mutable global property, matching `JSContext.eval_obj`.
    pub(crate) fn attach_eval_function(
        &mut self,
        realm: ContextId,
        function: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.eval_function.is_some() {
            return Err(HeapError::Invariant(
                "context already has an eval function root",
            ));
        }
        let function_object = self.object(function)?;
        if function_object.is_constructor
            || !matches!(
                function_object.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::GlobalEval,
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "eval function root is not the realm's non-constructor eval native",
            ));
        }

        self.retain_raw(RawId::Object(function), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining eval")
        };
        context.eval_function = Some(function);
        Ok(())
    }

    /// Publish the realm's `%Array%` root after its native callable and
    /// constructor/prototype cycle have been initialized.
    pub(crate) fn attach_array_constructor(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.array_constructor.is_some() {
            return Err(HeapError::Invariant(
                "context already has an Array constructor root",
            ));
        }
        let constructor_object = self.object(constructor)?;
        if !constructor_object.is_constructor
            || !matches!(
                constructor_object.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::ArrayConstructor,
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Array constructor root is not the realm's Array native",
            ));
        }

        self.retain_raw(RawId::Object(constructor), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Array")
        };
        context.array_constructor = Some(constructor);
        Ok(())
    }

    /// Atomically publish the realm's RegExp intrinsic roots after its native
    /// constructor, ordinary prototype, RegExp String Iterator prototype, and
    /// canonical instance shape exist.
    ///
    /// `last_index_atom` is validation-only: the shape already owns its atom
    /// edge. Passing it explicitly lets this heap layer prove that the sole
    /// instance slot really is `lastIndex` without depending on `AtomTable`
    /// string lookup. The four GC edges are retained as one transaction
    /// before the Context is mutated.
    pub(crate) fn attach_regexp_intrinsics(
        &mut self,
        realm: ContextId,
        regexp: RegExpRealmData,
        last_index_atom: Atom,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.regexp.is_some() {
            return Err(HeapError::Invariant(
                "context already has RegExp intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let constructor = self.object(regexp.constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::RegExp(RegExpNativeKind::Constructor),
                        realm: Some(target_realm),
                        ..
                    },
                    ..
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "RegExp constructor root is not the realm's RegExp native",
            ));
        }

        let prototype = self.object(regexp.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "RegExp prototype root is not an ordinary object",
            ));
        }

        let string_iterator_prototype = self.object(regexp.string_iterator_prototype)?;
        if string_iterator_prototype.kind != ObjectKind::Ordinary
            || !matches!(string_iterator_prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "RegExp String Iterator prototype root is not an ordinary object",
            ));
        }
        if self.shape(string_iterator_prototype.shape)?.prototype() != Some(iterator_prototype) {
            return Err(HeapError::Invariant(
                "RegExp String Iterator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let object_shape = self.shape(regexp.object_shape)?;
        if object_shape.prototype() != Some(regexp.prototype) {
            return Err(HeapError::Invariant(
                "RegExp object shape does not inherit from the realm's RegExp prototype",
            ));
        }
        let [last_index] = object_shape.entries() else {
            return Err(HeapError::Invariant(
                "RegExp object shape does not contain exactly one lastIndex property",
            ));
        };
        if last_index.atom != last_index_atom
            || last_index.flags != PropertyFlags::data(true, false, false)
        {
            return Err(HeapError::Invariant(
                "RegExp object shape has an invalid lastIndex property",
            ));
        }

        let edges = [
            RawId::Object(regexp.prototype),
            RawId::Object(regexp.constructor),
            RawId::Object(regexp.string_iterator_prototype),
            RawId::Shape(regexp.object_shape),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining RegExp roots")
        };
        context.regexp = Some(regexp);
        Ok(())
    }

    /// Atomically publish the realm's Map constructor, ordinary prototype,
    /// and Map Iterator prototype roots after all three have been initialized.
    pub(crate) fn attach_map_intrinsics(
        &mut self,
        realm: ContextId,
        map: MapRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.map.is_some() {
            return Err(HeapError::Invariant(
                "context already has Map intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let prototype = self.object(map.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Map prototype root is not an ordinary object",
            ));
        }

        let map_iterator_prototype = self.object(map.iterator_prototype)?;
        if map_iterator_prototype.kind != ObjectKind::Ordinary
            || !matches!(map_iterator_prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Map Iterator prototype root is not an ordinary object",
            ));
        }
        if self.shape(map_iterator_prototype.shape)?.prototype() != Some(iterator_prototype) {
            return Err(HeapError::Invariant(
                "Map Iterator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let edges = [
            RawId::Object(map.prototype),
            RawId::Object(map.iterator_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Map roots")
        };
        context.map = Some(map);
        Ok(())
    }

    /// Atomically publish the realm's ordinary Set prototype and Set Iterator
    /// prototype roots after both have been initialized.
    pub(crate) fn attach_set_intrinsics(
        &mut self,
        realm: ContextId,
        set: SetRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.set.is_some() {
            return Err(HeapError::Invariant(
                "context already has Set intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let prototype = self.object(set.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Set prototype root is not an ordinary object",
            ));
        }

        let set_iterator_prototype = self.object(set.iterator_prototype)?;
        if set_iterator_prototype.kind != ObjectKind::Ordinary
            || !matches!(set_iterator_prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "Set Iterator prototype root is not an ordinary object",
            ));
        }
        if self.shape(set_iterator_prototype.shape)?.prototype() != Some(iterator_prototype) {
            return Err(HeapError::Invariant(
                "Set Iterator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let edges = [
            RawId::Object(set.prototype),
            RawId::Object(set.iterator_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Set roots")
        };
        context.set = Some(set);
        Ok(())
    }

    /// Atomically publish the realm's ordinary WeakMap prototype root.
    pub(crate) fn attach_weak_map_intrinsics(
        &mut self,
        realm: ContextId,
        weak_map: WeakMapRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.weak_map.is_some() {
            return Err(HeapError::Invariant(
                "context already has WeakMap intrinsic roots",
            ));
        }
        let prototype = self.object(weak_map.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "WeakMap prototype root is not an ordinary object",
            ));
        }

        self.retain_raw(RawId::Object(weak_map.prototype), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining WeakMap roots")
        };
        context.weak_map = Some(weak_map);
        Ok(())
    }

    /// Atomically publish the realm's ordinary WeakSet prototype root.
    pub(crate) fn attach_weak_set_intrinsics(
        &mut self,
        realm: ContextId,
        weak_set: WeakSetRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.weak_set.is_some() {
            return Err(HeapError::Invariant(
                "context already has WeakSet intrinsic roots",
            ));
        }
        let prototype = self.object(weak_set.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
        {
            return Err(HeapError::Invariant(
                "WeakSet prototype root is not an ordinary object",
            ));
        }

        self.retain_raw(RawId::Object(weak_set.prototype), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining WeakSet roots")
        };
        context.weak_set = Some(weak_set);
        Ok(())
    }

    /// Atomically publish the realm's WeakRef and FinalizationRegistry class
    /// prototype roots. Both are ordinary children of this realm's
    /// Object.prototype and are installed together by pinned QuickJS.
    pub(crate) fn attach_weak_ref_intrinsics(
        &mut self,
        realm: ContextId,
        weak_ref: WeakRefRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.weak_ref.is_some() {
            return Err(HeapError::Invariant(
                "context already has WeakRef intrinsic roots",
            ));
        }
        if weak_ref.weak_ref_prototype == weak_ref.finalization_registry_prototype {
            return Err(HeapError::Invariant(
                "WeakRef and FinalizationRegistry prototypes share one identity",
            ));
        }
        let object_prototype = context.object_prototype;
        for (prototype, message) in [
            (
                weak_ref.weak_ref_prototype,
                "WeakRef prototype is not an ordinary child of Object.prototype",
            ),
            (
                weak_ref.finalization_registry_prototype,
                "FinalizationRegistry prototype is not an ordinary child of Object.prototype",
            ),
        ] {
            let object = self.object(prototype)?;
            if object.kind != ObjectKind::Ordinary
                || !matches!(object.payload, ObjectPayload::Ordinary)
                || self.shape(object.shape)?.prototype() != Some(object_prototype)
            {
                return Err(HeapError::Invariant(message));
            }
        }

        let edges = [
            RawId::Object(weak_ref.weak_ref_prototype),
            RawId::Object(weak_ref.finalization_registry_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining WeakRef roots")
        };
        context.weak_ref = Some(weak_ref);
        Ok(())
    }

    /// Atomically publish the realm's `%ArrayBuffer.prototype%` class root
    /// after validating the public constructor relationship.
    pub(crate) fn attach_array_buffer_intrinsics(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
        array_buffer: ArrayBufferRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.array_buffer.is_some() {
            return Err(HeapError::Invariant(
                "context already has ArrayBuffer intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;

        let constructor = self.object(constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::ArrayBuffer(
                            ArrayBufferNativeKind::Constructor
                        ),
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "ArrayBuffer constructor root is not the realm's ArrayBuffer native",
            ));
        }

        let prototype = self.object(array_buffer.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "ArrayBuffer prototype is not an ordinary child of Object.prototype",
            ));
        }

        let edges = [RawId::Object(array_buffer.prototype)];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining ArrayBuffer roots")
        };
        context.array_buffer = Some(array_buffer);
        Ok(())
    }

    /// Atomically publish the realm's `%SharedArrayBuffer.prototype%` class
    /// root after validating its independent public constructor relationship.
    pub(crate) fn attach_shared_array_buffer_intrinsics(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
        shared_array_buffer: SharedArrayBufferRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.shared_array_buffer.is_some() {
            return Err(HeapError::Invariant(
                "context already has SharedArrayBuffer intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;

        let constructor = self.object(constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::SharedArrayBuffer(
                            SharedArrayBufferNativeKind::Constructor
                        ),
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "SharedArrayBuffer constructor root is not the realm's SharedArrayBuffer native",
            ));
        }

        let prototype = self.object(shared_array_buffer.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "SharedArrayBuffer prototype is not an ordinary child of Object.prototype",
            ));
        }

        let edges = [RawId::Object(shared_array_buffer.prototype)];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining SharedArrayBuffer roots")
        };
        context.shared_array_buffer = Some(shared_array_buffer);
        Ok(())
    }

    /// Atomically publish the twelve concrete TypedArray class prototypes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn attach_typed_array_intrinsics(
        &mut self,
        realm: ContextId,
        base_constructor: ObjectId,
        base_prototype: ObjectId,
        constructors: [ObjectId; TypedArrayElementKind::COUNT],
        typed_array: TypedArrayRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.typed_array.is_some() {
            return Err(HeapError::Invariant(
                "context already has TypedArray intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;
        let function_prototype = context.function_prototype;

        let base_constructor_data = self.object(base_constructor)?;
        if !base_constructor_data.is_constructor
            || self.shape(base_constructor_data.shape)?.prototype() != Some(function_prototype)
            || !matches!(
                base_constructor_data.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::TypedArray(
                            TypedArrayNativeKind::BaseConstructor
                        ),
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "TypedArray base constructor has an invalid realm or prototype",
            ));
        }

        let base_prototype_data = self.object(base_prototype)?;
        if base_prototype_data.kind != ObjectKind::Ordinary
            || !matches!(base_prototype_data.payload, ObjectPayload::Ordinary)
            || self.shape(base_prototype_data.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "TypedArray base prototype is not an ordinary child of Object.prototype",
            ));
        }

        for (index, element) in TypedArrayElementKind::ALL.into_iter().enumerate() {
            let constructor = self.object(constructors[index])?;
            if !constructor.is_constructor
                || self.shape(constructor.shape)?.prototype() != Some(base_constructor)
                || !matches!(
                    constructor.payload,
                    ObjectPayload::NativeFunction {
                        data: NativeFunctionData {
                            target: NativeFunctionId::TypedArray(
                                TypedArrayNativeKind::Constructor(target_element)
                            ),
                            realm: Some(target_realm),
                            ..
                        },
                        internal: None,
                    } if target_realm == realm && target_element == element
                )
            {
                return Err(HeapError::Invariant(
                    "concrete TypedArray constructor has an invalid class graph",
                ));
            }
            let prototype = self.object(typed_array.prototypes[index])?;
            if prototype.kind != ObjectKind::Ordinary
                || !matches!(prototype.payload, ObjectPayload::Ordinary)
                || self.shape(prototype.shape)?.prototype() != Some(base_prototype)
            {
                return Err(HeapError::Invariant(
                    "concrete TypedArray prototype has an invalid class graph",
                ));
            }
        }

        let edges = typed_array
            .prototypes
            .into_iter()
            .map(RawId::Object)
            .collect::<Vec<_>>();
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining TypedArray roots")
        };
        context.typed_array = Some(typed_array);
        Ok(())
    }

    /// Atomically publish the realm's `%DataView.prototype%` class root after
    /// validating the public constructor relationship.
    pub(crate) fn attach_data_view_intrinsics(
        &mut self,
        realm: ContextId,
        constructor: ObjectId,
        data_view: DataViewRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.data_view.is_some() {
            return Err(HeapError::Invariant(
                "context already has DataView intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;

        let constructor = self.object(constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::DataView(DataViewNativeKind::Constructor),
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "DataView constructor root is not the realm's DataView native",
            ));
        }

        let prototype = self.object(data_view.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "DataView prototype is not an ordinary child of Object.prototype",
            ));
        }

        let edges = [RawId::Object(data_view.prototype)];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining DataView roots")
        };
        context.data_view = Some(data_view);
        Ok(())
    }

    /// Atomically publish the synchronous Iterator constructor and the hidden
    /// Iterator Concat/Helper/Wrap class prototypes.
    pub(crate) fn attach_iterator_intrinsics(
        &mut self,
        realm: ContextId,
        iterator: IteratorRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.iterator.is_some() {
            return Err(HeapError::Invariant(
                "context already has Iterator intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;

        let constructor = self.object(iterator.constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::IteratorConstructor,
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Iterator constructor root is not the realm's Iterator native",
            ));
        }

        if iterator.concat_prototype == iterator.helper_prototype
            || iterator.concat_prototype == iterator.wrap_prototype
            || iterator.helper_prototype == iterator.wrap_prototype
        {
            return Err(HeapError::Invariant(
                "Iterator hidden prototypes share an identity",
            ));
        }
        for (prototype, message) in [
            (
                iterator.concat_prototype,
                "Iterator Concat prototype is not an ordinary child of the realm's Iterator prototype",
            ),
            (
                iterator.helper_prototype,
                "Iterator Helper prototype is not an ordinary child of the realm's Iterator prototype",
            ),
            (
                iterator.wrap_prototype,
                "Iterator Wrap prototype is not an ordinary child of the realm's Iterator prototype",
            ),
        ] {
            let object = self.object(prototype)?;
            if object.kind != ObjectKind::Ordinary
                || !matches!(object.payload, ObjectPayload::Ordinary)
                || self.shape(object.shape)?.prototype() != Some(iterator_prototype)
            {
                return Err(HeapError::Invariant(message));
            }
        }

        let edges = [
            RawId::Object(iterator.constructor),
            RawId::Object(iterator.concat_prototype),
            RawId::Object(iterator.helper_prototype),
            RawId::Object(iterator.wrap_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Iterator roots")
        };
        context.iterator = Some(iterator);
        Ok(())
    }

    /// Atomically publish the realm's Promise constructor and ordinary
    /// prototype after their public constructor/prototype links exist.
    pub(crate) fn attach_promise_intrinsics(
        &mut self,
        realm: ContextId,
        promise: PromiseRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.promise.is_some() {
            return Err(HeapError::Invariant(
                "context already has Promise intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;

        let constructor = self.object(promise.constructor)?;
        if !constructor.is_constructor
            || !matches!(
                constructor.payload,
                ObjectPayload::NativeFunction {
                    data: NativeFunctionData {
                        target: NativeFunctionId::Promise(PromiseNativeKind::Constructor),
                        realm: Some(target_realm),
                        ..
                    },
                    internal: None,
                } if target_realm == realm
            )
        {
            return Err(HeapError::Invariant(
                "Promise constructor root is not the realm's Promise native",
            ));
        }

        let prototype = self.object(promise.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "Promise prototype is not an ordinary child of Object.prototype",
            ));
        }

        let edges = [
            RawId::Object(promise.prototype),
            RawId::Object(promise.constructor),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Promise roots")
        };
        context.promise = Some(promise);
        Ok(())
    }

    /// Atomically publish the two realm-local synchronous-generator class
    /// prototypes after their property graph has been initialized.
    pub(crate) fn attach_generator_intrinsics(
        &mut self,
        realm: ContextId,
        generator: GeneratorRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.generator.is_some() {
            return Err(HeapError::Invariant(
                "context already has Generator intrinsic roots",
            ));
        }
        let iterator_prototype = context.iterator_prototype;
        let function_prototype = context.function_prototype;

        let prototype = self.object(generator.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype() != Some(iterator_prototype)
        {
            return Err(HeapError::Invariant(
                "Generator prototype does not inherit from the realm's Iterator prototype",
            ));
        }

        let generator_function_prototype = self.object(generator.function_prototype)?;
        if generator_function_prototype.kind != ObjectKind::Ordinary
            || !matches!(
                generator_function_prototype.payload,
                ObjectPayload::Ordinary
            )
            || self.shape(generator_function_prototype.shape)?.prototype()
                != Some(function_prototype)
        {
            return Err(HeapError::Invariant(
                "GeneratorFunction prototype does not inherit from Function.prototype",
            ));
        }

        let edges = [
            RawId::Object(generator.prototype),
            RawId::Object(generator.function_prototype),
        ];
        self.retain_edges_transactionally(&edges)?;

        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining Generator roots")
        };
        context.generator = Some(generator);
        Ok(())
    }

    /// Atomically publish the realm-local `%AsyncFunction.prototype%` root
    /// after its hidden constructor/prototype property graph exists.
    pub(crate) fn attach_async_function_intrinsics(
        &mut self,
        realm: ContextId,
        async_function: AsyncFunctionRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.async_function.is_some() {
            return Err(HeapError::Invariant(
                "context already has AsyncFunction intrinsic roots",
            ));
        }
        let function_prototype = context.function_prototype;

        let async_function_prototype = self.object(async_function.function_prototype)?;
        if async_function_prototype.kind != ObjectKind::Ordinary
            || !matches!(async_function_prototype.payload, ObjectPayload::Ordinary)
            || self.shape(async_function_prototype.shape)?.prototype() != Some(function_prototype)
        {
            return Err(HeapError::Invariant(
                "AsyncFunction prototype does not inherit from Function.prototype",
            ));
        }

        self.retain_raw(RawId::Object(async_function.function_prototype), 1)?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining AsyncFunction roots")
        };
        context.async_function = Some(async_function);
        Ok(())
    }

    /// Atomically publish the realm-local async-iterator/generator graph.
    pub(crate) fn attach_async_generator_intrinsics(
        &mut self,
        realm: ContextId,
        async_generator: AsyncGeneratorRealmData,
    ) -> Result<(), HeapError> {
        let context = self.context(realm)?;
        if context.async_generator.is_some() {
            return Err(HeapError::Invariant(
                "context already has AsyncGenerator intrinsic roots",
            ));
        }
        let object_prototype = context.object_prototype;
        let function_prototype = context.function_prototype;

        let async_iterator = self.object(async_generator.async_iterator_prototype)?;
        if async_iterator.kind != ObjectKind::Ordinary
            || !matches!(async_iterator.payload, ObjectPayload::Ordinary)
            || self.shape(async_iterator.shape)?.prototype() != Some(object_prototype)
        {
            return Err(HeapError::Invariant(
                "AsyncIterator prototype does not inherit from Object.prototype",
            ));
        }
        let async_from_sync = self.object(async_generator.async_from_sync_iterator_prototype)?;
        if async_from_sync.kind != ObjectKind::Ordinary
            || !matches!(async_from_sync.payload, ObjectPayload::Ordinary)
            || self.shape(async_from_sync.shape)?.prototype()
                != Some(async_generator.async_iterator_prototype)
        {
            return Err(HeapError::Invariant(
                "AsyncFromSyncIterator prototype does not inherit from AsyncIterator prototype",
            ));
        }
        let prototype = self.object(async_generator.prototype)?;
        if prototype.kind != ObjectKind::Ordinary
            || !matches!(prototype.payload, ObjectPayload::Ordinary)
            || self.shape(prototype.shape)?.prototype()
                != Some(async_generator.async_iterator_prototype)
        {
            return Err(HeapError::Invariant(
                "AsyncGenerator prototype does not inherit from AsyncIterator prototype",
            ));
        }
        let function = self.object(async_generator.function_prototype)?;
        if function.kind != ObjectKind::Ordinary
            || !matches!(function.payload, ObjectPayload::Ordinary)
            || self.shape(function.shape)?.prototype() != Some(function_prototype)
        {
            return Err(HeapError::Invariant(
                "AsyncGeneratorFunction prototype does not inherit from Function.prototype",
            ));
        }

        self.retain_edges_transactionally(&[
            RawId::Object(async_generator.async_iterator_prototype),
            RawId::Object(async_generator.async_from_sync_iterator_prototype),
            RawId::Object(async_generator.prototype),
            RawId::Object(async_generator.function_prototype),
        ])?;
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(realm))?.data
        else {
            unreachable!("context identity was validated before retaining AsyncGenerator roots")
        };
        context.async_generator = Some(async_generator);
        Ok(())
    }
}

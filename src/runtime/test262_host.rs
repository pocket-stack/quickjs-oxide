//! QuickJS-shaped Test262 realm host helpers.
//!
//! These functions are deliberately opt-in rather than ECMAScript intrinsics.
//! Each installed native function retains its defining realm, so a `$262`
//! object returned by `createRealm` keeps the child context alive after the
//! temporary Rust [`Context`] handle is dropped.

use super::*;

const EVAL_SCRIPT_FILENAME: &str = "<evalScript>";

impl Runtime {
    fn test262_host_data_property(value: Value) -> OrdinaryPropertyDescriptor {
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        }
    }

    fn define_test262_host_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        if !self.define_own_property(object, &key, &Self::test262_host_data_property(value))? {
            return Err(RuntimeError::Invariant(
                "Test262 host property definition was rejected",
            ));
        }
        Ok(())
    }

    fn install_test262_host_in_realm(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let (object_prototype, function_prototype, global_object) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            (
                context.object_prototype,
                context.function_prototype,
                context.global_object,
            )
        };
        let object_prototype = ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?;
        let function_prototype = ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;

        // Match pinned run-test262's ordinary `$262` object and C/W/E
        // JS_SetPropertyStr property shape. Agent and IsHTMLDDA remain separate
        // optional host capabilities and are intentionally absent here.
        let object_262 = self.new_object(Some(&object_prototype))?;
        let detach_array_buffer = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::Test262DetachArrayBuffer,
            1,
            "detachArrayBuffer",
            1,
        )?;
        self.define_test262_host_property(
            &object_262,
            "detachArrayBuffer",
            Value::Object(detach_array_buffer.as_object().clone()),
        )?;

        let eval_script = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::Test262EvalScript,
            1,
            "evalScript",
            1,
        )?;
        self.define_test262_host_property(
            &object_262,
            "evalScript",
            Value::Object(eval_script.as_object().clone()),
        )?;

        let code_point_range = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::StringCodePointRange,
            2,
            "codePointRange",
            2,
        )?;
        self.define_test262_host_property(
            &object_262,
            "codePointRange",
            Value::Object(code_point_range.as_object().clone()),
        )?;

        // Pinned QuickJS installs `agent` at this exact point: after
        // codePointRange and before global. The optional session binding keeps
        // ordinary embedders and non-agent Test262 runs unchanged.
        if let Some(agent) = self.new_registered_test262_agent_object(realm)? {
            self.define_test262_host_property(&object_262, "agent", Value::Object(agent))?;
        }

        self.define_test262_host_property(
            &object_262,
            "global",
            Value::Object(global_object.clone()),
        )?;

        let create_realm = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::Test262CreateRealm,
            0,
            "createRealm",
            0,
        )?;
        self.define_test262_host_property(
            &object_262,
            "createRealm",
            Value::Object(create_realm.as_object().clone()),
        )?;

        let gc = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::Test262Gc,
            0,
            "gc",
            0,
        )?;
        self.define_test262_host_property(
            &object_262,
            "gc",
            Value::Object(gc.as_object().clone()),
        )?;

        self.define_test262_host_property(
            &global_object,
            "$262",
            Value::Object(object_262.clone()),
        )?;
        Ok(object_262)
    }

    pub(in crate::runtime) fn call_test262_eval_script(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 evalScript received a constructor invocation",
            ));
        };
        let source = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Test262 evalScript argument was not padded",
        ))?;
        let source = match self.native_to_js_string(realm, source)? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        // The compiler currently accepts UTF-8 source rather than an exact
        // UTF-16 code-unit stream. Reject an unpaired surrogate explicitly;
        // lossy replacement would silently evaluate different JavaScript.
        let source_units = source.utf16_units().collect::<Vec<_>>();
        let source = match String::from_utf16(&source_units) {
            Ok(source) => source,
            Err(_) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "evalScript source containing a lone UTF-16 surrogate is not implemented",
                )?));
            }
        };

        let script = match self.compile_in_realm(realm, &source, EVAL_SCRIPT_FILENAME, false)? {
            Compilation::Published(script) => script,
            Compilation::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callable = self.new_bytecode_closure(realm, &script)?;
        let global_object = self.global_object_for_realm(realm)?;
        self.call_internal(realm, &callable, Value::Object(global_object), &[])
    }

    pub(in crate::runtime) fn call_test262_create_realm(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Test262 createRealm received a constructor invocation",
            ));
        };
        let mut child = self.new_context();
        let object_262 = child.install_test262_host()?;
        drop(child);
        Ok(Completion::Return(Value::Object(object_262)))
    }
}

impl Context {
    /// Install the QuickJS Test262 host surface in this realm and return its
    /// ordinary `$262` object.
    ///
    /// Every installed property, including `global.$262`, is writable,
    /// enumerable, and configurable, matching pinned QuickJS's
    /// `JS_SetPropertyStr` behavior. Calling the installed `createRealm`
    /// recursively installs the same surface in a fresh context belonging to
    /// this [`Runtime`].
    pub fn install_test262_host(&mut self) -> Result<ObjectRef, RuntimeError> {
        self.runtime.install_test262_host_in_realm(self.realm)
    }
}

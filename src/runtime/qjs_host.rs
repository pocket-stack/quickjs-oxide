//! Optional qjs command-line host functions.
//!
//! These are deliberately installed by the binary, not by `Runtime::new_context`:
//! `print` belongs to the qjs host surface and is not an ECMAScript intrinsic.

use super::*;

impl Runtime {
    pub(in crate::runtime) fn call_qjs_print(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "qjs print received a constructor invocation",
            ));
        };
        let mut fields = Vec::with_capacity(arguments.actual_arg_count);
        for argument in &arguments.readable[..arguments.actual_arg_count] {
            let field = match self.native_to_js_string(realm, argument)? {
                NativeConversion::Value(field) => field,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            fields.push(field.to_utf8_lossy());
        }
        println!("{}", fields.join(" "));
        Ok(Completion::Return(Value::Undefined))
    }
}

impl Context {
    /// Install qjs's host-provided `print` function on this realm's global.
    /// Embedders which need a pure ECMAScript realm simply do not call this.
    pub fn install_qjs_print(&mut self) -> Result<(), RuntimeError> {
        let function_prototype = self.function_prototype()?;
        let global = self.global_object()?;
        let print = self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::QjsPrint,
            0,
            "print",
            1,
        )?;
        self.runtime.define_function_data_property(
            &global,
            "print",
            Value::Object(print.as_object().clone()),
            true,
            true,
        )
    }
}

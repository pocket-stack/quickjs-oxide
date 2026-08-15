//! Optional qjs command-line host functions.
//!
//! These are deliberately installed by the binary, not by `Runtime::new_context`:
//! `print` and `console.log` belong to the qjs host surface and are not
//! ECMAScript intrinsics.

use super::*;
use std::io::{self, Write};

impl Runtime {
    #[inline(never)]
    pub(in crate::runtime) fn call_qjs_output(
        &self,
        target: NativeFunctionId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let flush = match target {
            NativeFunctionId::QjsPrint => false,
            NativeFunctionId::QjsConsoleLog => true,
            _ => {
                return Err(RuntimeError::Invariant(
                    "non-qjs native reached the qjs output dispatcher",
                ));
            }
        };
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "qjs output helper received a constructor invocation",
            ));
        };

        let mut line = Vec::new();
        for (index, argument) in arguments.readable[..arguments.actual_arg_count]
            .iter()
            .enumerate()
        {
            if index != 0 {
                line.push(b' ');
            }
            if let Value::String(value) = argument {
                value.try_append_wtf8_bytes(&mut line).map_err(|_| {
                    RuntimeError::Engine(Error::new(
                        ErrorKind::Internal,
                        "qjs print could not allocate a String byte buffer",
                    ))
                })?;
            } else {
                self.qjs_print_value_into_bytes(argument, &mut line)?;
            }
        }
        line.push(b'\n');

        // Upstream deliberately ignores fwrite/putchar/fflush failures. Keep
        // host I/O outside JavaScript completion semantics for exact parity.
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        let _ = stdout.write_all(&line);
        if flush {
            let _ = stdout.flush();
        }
        Ok(Completion::Return(Value::Undefined))
    }
}

impl Context {
    /// Install qjs's host-provided `print` function on this realm's global.
    /// Embedders which need a pure ECMAScript realm simply do not call this.
    pub fn install_qjs_print(&mut self) -> Result<(), RuntimeError> {
        self.install_qjs_print_function()
    }

    /// Install qjs's ordinary command-line output helpers. This preserves the
    /// upstream publication order (`console` before `print`) and leaves pure
    /// embedder-created realms untouched.
    pub fn install_qjs_helpers(&mut self) -> Result<(), RuntimeError> {
        let function_prototype = self.function_prototype()?;
        let object_prototype = self.object_prototype()?;
        let global = self.global_object()?;
        let console = self.runtime.new_object(Some(&object_prototype))?;
        let log = self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::QjsConsoleLog,
            0,
            "log",
            1,
        )?;
        self.define_qjs_host_property(&console, "log", Value::Object(log.as_object().clone()))?;
        self.define_qjs_host_property(&global, "console", Value::Object(console))?;
        self.install_qjs_print_function()
    }

    fn install_qjs_print_function(&mut self) -> Result<(), RuntimeError> {
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
        self.define_qjs_host_property(&global, "print", Value::Object(print.as_object().clone()))
    }

    fn define_qjs_host_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.runtime.intern_property_key(name)?;
        if !self.runtime.define_own_property(
            object,
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
                "qjs host property definition was rejected",
            ));
        }
        Ok(())
    }
}

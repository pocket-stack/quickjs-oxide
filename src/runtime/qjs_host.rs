//! Optional qjs command-line host functions.
//!
//! These are deliberately installed by the binary, not by `Runtime::new_context`:
//! `print`, `console.log`, and `scriptArgs` belong to the qjs host surface and
//! are not ECMAScript intrinsics. This implemented subset follows the relative
//! publication order in `quickjs-libc.c::js_std_add_helpers`; qjs passes that
//! helper the unconsumed argv tail selected by `qjs.c::main`.

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

    /// Install qjs's ordinary command-line output helpers without
    /// `scriptArgs`.
    ///
    /// This matches the helper subset used by non-CLI hosts. The qjs binary
    /// instead calls [`Self::install_qjs_helpers_with_script_args`], including
    /// with an empty slice when no argv entries remain.
    pub fn install_qjs_helpers(&mut self) -> Result<(), RuntimeError> {
        self.install_qjs_helpers_inner(None)
    }

    /// Install qjs's ordinary command-line helpers and publish the exact
    /// remaining process arguments as `scriptArgs`.
    ///
    /// The qjs binary passes the main filename followed by its arguments for
    /// file evaluation, or only the arguments remaining after `-e` for eval
    /// mode. Keeping the already-decoded Strings at this host boundary mirrors
    /// `js_std_add_helpers` without making `scriptArgs` an ECMAScript
    /// intrinsic in embedder-created realms.
    pub fn install_qjs_helpers_with_script_args(
        &mut self,
        script_args: &[JsString],
    ) -> Result<(), RuntimeError> {
        self.install_qjs_helpers_inner(Some(script_args))
    }

    fn install_qjs_helpers_inner(
        &mut self,
        script_args: Option<&[JsString]>,
    ) -> Result<(), RuntimeError> {
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
        let arguments = script_args
            .map(|script_args| {
                self.new_array_from_values(script_args.iter().cloned().map(Value::String).collect())
            })
            .transpose()?;
        let print = self.new_qjs_print_function(&function_prototype)?;

        // Finish every fallible value allocation before exposing the first
        // helper on the fresh qjs realm. Property publication itself follows
        // js_std_add_helpers: console, optional scriptArgs, then print.
        self.define_qjs_host_property(&console, "log", Value::Object(log.as_object().clone()))?;
        self.define_qjs_host_property(&global, "console", Value::Object(console))?;
        if let Some(arguments) = arguments {
            self.define_qjs_host_property(&global, "scriptArgs", Value::Object(arguments))?;
        }
        self.define_qjs_host_property(&global, "print", Value::Object(print.as_object().clone()))
    }

    fn install_qjs_print_function(&mut self) -> Result<(), RuntimeError> {
        let function_prototype = self.function_prototype()?;
        let global = self.global_object()?;
        let print = self.new_qjs_print_function(&function_prototype)?;
        self.define_qjs_host_property(&global, "print", Value::Object(print.as_object().clone()))
    }

    fn new_qjs_print_function(
        &self,
        function_prototype: &ObjectRef,
    ) -> Result<CallableRef, RuntimeError> {
        self.runtime.new_native_builtin(
            function_prototype,
            self.realm,
            NativeFunctionId::QjsPrint,
            0,
            "print",
            1,
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_args_distinguishes_non_cli_helpers_from_an_empty_cli_tail() {
        let runtime = Runtime::new();

        let mut embedder = runtime.new_context();
        embedder.install_qjs_helpers().unwrap();
        assert_eq!(
            embedder.eval("typeof scriptArgs").unwrap(),
            Value::String(JsString::from_static("undefined"))
        );

        let mut cli = runtime.new_context();
        cli.install_qjs_helpers_with_script_args(&[]).unwrap();
        assert_eq!(
            cli.eval("Array.isArray(scriptArgs) && scriptArgs.length === 0")
                .unwrap(),
            Value::Bool(true)
        );
    }
}

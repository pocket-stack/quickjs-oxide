//! Read and publish the existing trusted bytecode compatibility profiles.

use super::*;

impl Context {
    /// Read one trusted QuickJS 2026-06-04 BC5 scalar Script.
    ///
    /// This deliberately narrow compatibility entry point is not a general
    /// untrusted-bytecode sandbox. It accepts only the currently admitted
    /// branch-free scalar Script cohort, translates it to typed engine
    /// instructions, and runs the ordinary verifier before publication.
    /// Well-formed QuickJS bytecode outside that cohort returns
    /// [`ErrorKind::Unsupported`] without creating a pending exception.
    pub fn read_trusted_scalar_script(
        &mut self,
        bytes: &[u8],
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let result = self
            .runtime
            .read_trusted_scalar_script_in_realm(self.realm, bytes);
        self.finish_trusted_bytecode_read(result)
    }

    /// Read one ordinary synchronous leaf from a trusted QuickJS 2026-06-04
    /// BC5 compile-only image.
    ///
    /// `root_constant_index` selects a FunctionBytecode child from the
    /// authenticated root function's constant pool. The selected child must
    /// satisfy the current ordinary-leaf capability profile; its complete
    /// typed CFG is verified before transactional publication. Well-formed
    /// bytecode outside that profile returns [`ErrorKind::Unsupported`]
    /// without creating a pending exception.
    pub fn read_trusted_ordinary_function(
        &mut self,
        bytes: &[u8],
        root_constant_index: u32,
    ) -> Result<CallableRef, RuntimeError> {
        let result = self.runtime.read_trusted_ordinary_function_in_realm(
            self.realm,
            bytes,
            root_constant_index,
        );
        self.finish_trusted_bytecode_read(result)
    }

    fn finish_trusted_bytecode_read<T>(
        &mut self,
        result: Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        match result {
            Ok(function) => Ok(function),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible bytecode read error");
                let exception = self
                    .runtime
                    .new_native_error_from_error(self.realm, kind, &error)?;
                self.runtime
                    .ensure_error_backtrace(&exception, false, None)?;
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            Err(error) => Err(error),
        }
    }
}

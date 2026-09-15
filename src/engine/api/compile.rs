//! Source request orchestration and JavaScript compilation error boundaries.

use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::debug::DebugInfoMode;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::compiler::{
    compile_unlinked_script_bytes_with_filename, compile_unlinked_script_with_filename,
};
use crate::engine::heap::ContextId;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::frames::ExplicitBacktraceLocation;
use crate::source::QuickJsSourceLocator;

pub(crate) enum Compilation {
    Published(FunctionBytecodeRef),
    Throw(Value),
}

impl Runtime {
    /// Compile and publish source without mutating the runtime pending-
    /// exception slot. Native indirect-eval paths need the thrown value as a
    /// normal completion, while the public Context boundary installs that
    /// same value into the pending slot before returning `Exception`.
    pub(crate) fn compile_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
    ) -> Result<Compilation, RuntimeError> {
        self.compile_script_in_realm(
            realm,
            QuickJsSourceLocator::new(source),
            filename,
            |debug_info| compile_unlinked_script_with_filename(source, filename, debug_info),
        )
    }

    /// Construct, compile, and publish one explicitly sized source buffer
    /// inside the same exception boundary as ordinary UTF-8 source.
    pub(crate) fn compile_bytes_in_realm(
        &self,
        realm: ContextId,
        source: &[u8],
        filename: &str,
    ) -> Result<Compilation, RuntimeError> {
        self.compile_script_in_realm(
            realm,
            QuickJsSourceLocator::from_bytes(source),
            filename,
            |debug_info| compile_unlinked_script_bytes_with_filename(source, filename, debug_info),
        )
    }

    pub(crate) fn compile_script_in_realm(
        &self,
        realm: ContextId,
        source_locator: QuickJsSourceLocator<'_>,
        filename: &str,
        compile: impl FnOnce(DebugInfoMode) -> Result<UnlinkedFunction, Error>,
    ) -> Result<Compilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let function = match compile(debug_info) {
            Ok(function) => function,
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                let explicit_location = if error.kind() == ErrorKind::Syntax {
                    if let Some(span) = error.span() {
                        let position = source_locator
                            .locate_byte_offset(span.start.byte_offset)
                            .map_err(|_| {
                                RuntimeError::Invariant(
                                    "syntax-error byte offset is invalid for its source",
                                )
                            })?;
                        Some(ExplicitBacktraceLocation {
                            filename: JsString::try_from_utf8(filename)?,
                            position,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let exception = if error.kind() == ErrorKind::Syntax {
                    self.new_native_error_without_backtrace_from_error(realm, kind, &error)?
                } else {
                    self.new_native_error_from_error(realm, kind, &error)?
                };
                self.ensure_error_backtrace(&exception, false, explicit_location)?;
                return Ok(Compilation::Throw(exception));
            }
        };
        Ok(Compilation::Published(
            self.publish_unlinked_function(realm, function)?,
        ))
    }
}

/// Adapt a compilation request to the narrower publication authority view.
pub(crate) fn eval_publication_input(
    context: &crate::engine::compiler::EvalCompileContext,
) -> crate::engine::code::function::publication::EvalPublicationInput<'_> {
    crate::engine::code::function::publication::EvalPublicationInput {
        kind: context.kind,
        caller_strict: context.caller_strict,
        bindings: &context.bindings,
        caller_profile: &context.caller_profile,
        super_call_allowed: context.super_call_allowed,
        super_allowed: context.super_allowed,
        arguments_forbidden: context.arguments_forbidden,
    }
}

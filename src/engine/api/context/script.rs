//! Script compilation and evaluation, including execution options.

use super::*;

/// Execution-only eval options. Compilation metadata remains in
/// [`CompileOptions`]; the barrier mirrors QuickJS
/// `JS_EVAL_FLAG_BACKTRACE_BARRIER` and temporarily marks the caller frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalOptions {
    pub filename: String,
    pub backtrace_barrier: bool,
}

impl EvalOptions {
    #[must_use]
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            backtrace_barrier: false,
        }
    }
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self::new(crate::engine::compiler::DEFAULT_EVAL_FILENAME)
    }
}

impl Context {
    /// Compile one script and publish its immutable bytecode in this realm.
    ///
    /// The returned handle is a runtime root. Its constant pool and captured
    /// realm remain alive even if this particular `Context` handle is dropped.
    pub fn compile(&mut self, source: &str) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::default())
    }

    /// Compile one explicitly sized source buffer. Full debug mode retains
    /// byte-exact authored ranges for nested functions.
    pub fn compile_bytes(&mut self, source: &[u8]) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_bytes_with_options(source, &CompileOptions::default())
    }

    /// Compile one script with an explicit filename attached independently to
    /// every published function's debug metadata.
    pub fn compile_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one explicitly sized source buffer with an explicit debug
    /// filename.
    pub fn compile_bytes_with_filename(
        &mut self,
        source: &[u8],
        filename: &str,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_bytes_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one script with named compilation options.
    ///
    /// Implemented JavaScript early errors become pending exceptions. Grammar
    /// which is not implemented remains an engine [`ErrorKind::Unsupported`]
    /// diagnostic so embedders and conformance tooling observe the same
    /// frontier.
    pub fn compile_with_options(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let compilation = self
            .runtime
            .compile_in_realm(self.realm, source, &options.filename)?;
        self.finish_compilation(compilation)
    }

    /// Compile one explicitly sized source buffer with named compilation
    /// options.
    ///
    /// Source bytes are parsed with QuickJS-compatible UTF-8/WTF-8 handling;
    /// malformed bytes remain observable in permitted source regions and
    /// produce syntax errors where the grammar requires source characters.
    pub fn compile_bytes_with_options(
        &mut self,
        source: &[u8],
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let compilation =
            self.runtime
                .compile_bytes_in_realm(self.realm, source, &options.filename)?;
        self.finish_compilation(compilation)
    }

    fn finish_compilation(
        &mut self,
        compilation: Compilation,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        match compilation {
            Compilation::Published(function) => Ok(function),
            Compilation::Throw(exception) => {
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Compile and evaluate one script through runtime-owned bytecode.
    ///
    /// # Errors
    /// Returns syntax, publication, runtime-domain, or execution errors.
    pub fn eval(&mut self, source: &str) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::default())
    }

    /// Compile and evaluate one explicitly sized source buffer.
    pub fn eval_bytes(&mut self, source: &[u8]) -> Result<Value, RuntimeError> {
        self.eval_bytes_with_options(source, &EvalOptions::default())
    }

    /// Compile and evaluate a script with an explicit debug filename.
    pub fn eval_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::new(filename))
    }

    /// Compile and evaluate one explicitly sized source buffer with an
    /// explicit debug filename.
    pub fn eval_bytes_with_filename(
        &mut self,
        source: &[u8],
        filename: &str,
    ) -> Result<Value, RuntimeError> {
        self.eval_bytes_with_options(source, &EvalOptions::new(filename))
    }

    /// Compile and evaluate a script with filename and execution options.
    pub fn eval_with_options(
        &mut self,
        source: &str,
        options: &EvalOptions,
    ) -> Result<Value, RuntimeError> {
        self.eval_compiling_with_options(options, |context, compile_options| {
            context.compile_with_options(source, compile_options)
        })
    }

    /// Compile and evaluate one explicitly sized source buffer with filename
    /// and execution options.
    pub fn eval_bytes_with_options(
        &mut self,
        source: &[u8],
        options: &EvalOptions,
    ) -> Result<Value, RuntimeError> {
        self.eval_compiling_with_options(options, |context, compile_options| {
            context.compile_bytes_with_options(source, compile_options)
        })
    }

    fn eval_compiling_with_options(
        &mut self,
        options: &EvalOptions,
        compile: impl FnOnce(&mut Self, &CompileOptions) -> Result<FunctionBytecodeRef, RuntimeError>,
    ) -> Result<Value, RuntimeError> {
        let barrier = self
            .runtime
            .install_backtrace_barrier(options.backtrace_barrier)?;
        let result = (|| {
            let function = compile(self, &CompileOptions::new(&options.filename))?;
            self.execute(&function)
        })();
        barrier.finish()?;
        result
    }
}

//! Feature-gated constructors used by the Test262 host.

use super::*;

impl Context {
    /// Create QuickJS's test262-only native `codePointRange` helper in this
    /// context's realm.
    ///
    /// The helper is intentionally not installed as an ECMAScript intrinsic;
    /// embedders such as the Test262 runner decide where to publish it.
    #[cfg(feature = "test262-host")]
    pub fn new_code_point_range_function(&mut self) -> Result<CallableRef, RuntimeError> {
        let function_prototype = self.function_prototype()?;
        self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::StringCodePointRange,
            2,
            "codePointRange",
            2,
        )
    }

    /// Create QuickJS's test262-only `$262.gc` host function.
    ///
    /// The function is not an ECMAScript intrinsic. Embedders choose whether
    /// and where to publish it.
    #[cfg(feature = "test262-host")]
    pub fn new_test262_gc_function(&mut self) -> Result<CallableRef, RuntimeError> {
        let function_prototype = self.function_prototype()?;
        self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::Test262Gc,
            0,
            "gc",
            0,
        )
    }
}

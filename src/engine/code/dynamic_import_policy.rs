#[cfg(feature = "test262-host")]
use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::rooted::FunctionBytecodeRef;

#[cfg(feature = "test262-host")]
use crate::engine::heap::{BytecodeConstant, FunctionBytecodeId};
#[cfg(feature = "test262-host")]
use std::cell::Cell;
#[cfg(feature = "test262-host")]
use std::collections::HashSet;

impl Runtime {
    /// Inspect one immutable function tree for the dynamic-import opcode.
    ///
    /// This non-default host-support surface traverses published child
    /// function constants, avoiding source-text guesses about executable
    /// syntax while leaving ordinary embedders' API surface unchanged.
    #[cfg(feature = "test262-host")]
    pub fn bytecode_tree_contains_dynamic_import(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<bool, RuntimeError> {
        self.dynamic_import_bytecode_tree_contains(function)
    }

    #[cfg(feature = "test262-host")]
    pub(crate) fn dynamic_import_bytecode_tree_contains(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<bool, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        self.dynamic_import_bytecode_id_tree_contains(function.bytecode_id())
    }

    #[cfg(feature = "test262-host")]
    pub(crate) fn dynamic_import_bytecode_id_tree_contains(
        &self,
        function: FunctionBytecodeId,
    ) -> Result<bool, RuntimeError> {
        let state = self.0.state.borrow();
        let mut pending = vec![function];
        let mut visited = HashSet::new();
        while let Some(bytecode) = pending.pop() {
            if !visited.insert(bytecode) {
                continue;
            }
            let bytecode = state.heap.function_bytecode(bytecode)?;
            if bytecode.code.iter().any(|instruction| {
                matches!(
                    instruction,
                    crate::engine::code::bytecode::Instruction::Import
                )
            }) {
                return Ok(true);
            }
            pending.extend(
                bytecode
                    .constants
                    .iter()
                    .filter_map(|constant| match constant {
                        BytecodeConstant::Function(child) => Some(*child),
                        BytecodeConstant::Value(_) | BytecodeConstant::RegExp { .. } => None,
                    }),
            );
        }
        Ok(false)
    }

    /// Set the dynamic-import capability for this isolated runtime.
    ///
    /// This non-default host-support surface lets a conformance runner keep
    /// harness and unauthenticated programs fail-closed, then enable the
    /// capability only after its external admission checks succeed. A fresh
    /// runtime starts enabled so ordinary feature-enabled embedders retain the
    /// same JavaScript semantics as default builds.
    #[cfg(feature = "test262-host")]
    pub fn set_dynamic_import_bytecode_allowed(&self, allowed: bool) {
        self.0.dynamic_import_bytecode_allowed.set(allowed);
    }

    /// Run one host operation with a temporary dynamic-import bytecode policy.
    ///
    /// The previous policy is restored on every return and during unwinding,
    /// so a failed compiler or module-loader callback cannot leave an
    /// unauthenticated runtime capability enabled.
    #[cfg(feature = "test262-host")]
    pub fn with_dynamic_import_bytecode_allowed<T>(
        &self,
        allowed: bool,
        operation: impl FnOnce() -> T,
    ) -> T {
        struct RestoreDynamicImportPolicy<'a> {
            policy: &'a Cell<bool>,
            previous: bool,
        }

        impl Drop for RestoreDynamicImportPolicy<'_> {
            fn drop(&mut self) {
                self.policy.set(self.previous);
            }
        }

        let previous = self.0.dynamic_import_bytecode_allowed.replace(allowed);
        let _restore = RestoreDynamicImportPolicy {
            policy: &self.0.dynamic_import_bytecode_allowed,
            previous,
        };
        operation()
    }

    #[cfg(feature = "test262-host")]
    pub(crate) fn ensure_dynamic_import_bytecode_tree_authorized(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<(), RuntimeError> {
        if self.0.dynamic_import_bytecode_allowed.get() {
            return Ok(());
        }
        if self.dynamic_import_bytecode_tree_contains(function)? {
            return Err(Error::internal(
                "host dynamic-import bytecode policy rejected a disabled executable",
            )
            .into());
        }
        Ok(())
    }

    #[cfg(not(feature = "test262-host"))]
    pub(crate) fn ensure_dynamic_import_bytecode_tree_authorized(
        &self,
        _function: &FunctionBytecodeRef,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }

    #[cfg(feature = "test262-host")]
    pub(crate) fn ensure_dynamic_import_bytecode_authorized(
        &self,
        _function: Option<&FunctionBytecodeRef>,
    ) -> Result<(), RuntimeError> {
        if self.0.dynamic_import_bytecode_allowed.get() {
            return Ok(());
        }
        Err(Error::internal("host dynamic-import bytecode policy rejected execution").into())
    }

    #[cfg(not(feature = "test262-host"))]
    pub(crate) fn ensure_dynamic_import_bytecode_authorized(
        &self,
        _function: Option<&FunctionBytecodeRef>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}

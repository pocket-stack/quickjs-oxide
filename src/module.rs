//! Runtime-independent ECMAScript module drafts.
//!
//! QuickJS keeps a `JSModuleDef` beside the compiled module function.  This
//! module is the Rust ownership boundary for the same information: parsing
//! produces request/import/export tables whose local binding indices point at
//! authenticated closure slots on the module root function.  Runtime
//! publication consumes the complete draft transactionally.

use crate::function::UnlinkedFunction;
use crate::value::JsString;

/// Source-order index into [`UnlinkedModule::requested_modules`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModuleRequestIndex(pub(crate) u32);

/// One normalized-later module specifier from an import or re-export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleRequest {
    pub(crate) specifier: JsString,
}

/// One imported binding linked to a root closure slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleImport {
    pub(crate) request: ModuleRequestIndex,
    pub(crate) import_name: ModuleImportName,
    pub(crate) closure_index: u16,
}

/// The target selected from one requested module.
///
/// QuickJS represents the namespace case with its private `JS_ATOM__star_`
/// atom. Keeping it outside the JavaScript string domain prevents a real
/// exported name `"*"` from being mistaken for that implementation sentinel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleImportName {
    Name(JsString),
    Namespace,
}

/// Compiler-authored value installed into one hoisted module declaration
/// during the synthetic link entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleLinkInitializerValue {
    Undefined,
    Function(u32),
}

/// One exact link-entry write to a non-lexical module declaration cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModuleLinkInitializer {
    pub(crate) closure_index: u16,
    pub(crate) value: ModuleLinkInitializerValue,
}

/// Resolved target of one exported name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModuleExportTarget {
    /// Exact root closure slot for a local declaration or imported binding.
    Local { closure_index: u16 },
    /// Named re-export resolved through a requested module.
    Indirect {
        request: ModuleRequestIndex,
        import_name: ModuleImportName,
    },
}

/// One non-star export. Export names are stored as JavaScript strings because
/// current ECMAScript permits string-literal names in import/export clauses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleExport {
    pub(crate) export_name: JsString,
    pub(crate) target: ModuleExportTarget,
}

/// One `export * from` edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModuleStarExport {
    pub(crate) request: ModuleRequestIndex,
}

/// Mutable compiler output which has not entered a runtime domain yet.
#[derive(Debug)]
pub(crate) struct UnlinkedModule {
    name: JsString,
    function: UnlinkedFunction,
    link_initializers: Box<[ModuleLinkInitializer]>,
    requested_modules: Box<[ModuleRequest]>,
    imports: Box<[ModuleImport]>,
    exports: Box<[ModuleExport]>,
    star_exports: Box<[ModuleStarExport]>,
}

/// Owned pieces crossing the one-way module publication boundary.
pub(crate) struct UnlinkedModuleParts {
    pub(crate) name: JsString,
    pub(crate) function: UnlinkedFunction,
    pub(crate) link_initializers: Box<[ModuleLinkInitializer]>,
    pub(crate) requested_modules: Box<[ModuleRequest]>,
    pub(crate) imports: Box<[ModuleImport]>,
    pub(crate) exports: Box<[ModuleExport]>,
    pub(crate) star_exports: Box<[ModuleStarExport]>,
}

impl UnlinkedModule {
    #[must_use]
    pub(crate) fn new(
        name: JsString,
        function: UnlinkedFunction,
        link_initializers: Vec<ModuleLinkInitializer>,
        requested_modules: Vec<ModuleRequest>,
        imports: Vec<ModuleImport>,
        exports: Vec<ModuleExport>,
        star_exports: Vec<ModuleStarExport>,
    ) -> Self {
        Self {
            name,
            function,
            link_initializers: link_initializers.into_boxed_slice(),
            requested_modules: requested_modules.into_boxed_slice(),
            imports: imports.into_boxed_slice(),
            exports: exports.into_boxed_slice(),
            star_exports: star_exports.into_boxed_slice(),
        }
    }

    #[must_use]
    pub(crate) const fn function(&self) -> &UnlinkedFunction {
        &self.function
    }

    #[must_use]
    pub(crate) const fn link_initializers(&self) -> &[ModuleLinkInitializer] {
        &self.link_initializers
    }

    #[must_use]
    pub(crate) const fn requested_modules(&self) -> &[ModuleRequest] {
        &self.requested_modules
    }

    #[must_use]
    pub(crate) const fn imports(&self) -> &[ModuleImport] {
        &self.imports
    }

    #[must_use]
    pub(crate) const fn exports(&self) -> &[ModuleExport] {
        &self.exports
    }

    #[must_use]
    pub(crate) const fn star_exports(&self) -> &[ModuleStarExport] {
        &self.star_exports
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> UnlinkedModuleParts {
        UnlinkedModuleParts {
            name: self.name,
            function: self.function,
            link_initializers: self.link_initializers,
            requested_modules: self.requested_modules,
            imports: self.imports,
            exports: self.exports,
            star_exports: self.star_exports,
        }
    }
}

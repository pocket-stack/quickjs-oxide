//! Runtime-independent ECMAScript module drafts.
//!
//! QuickJS keeps a `JSModuleDef` beside the compiled module function.  This
//! module is the Rust ownership boundary for the same information: parsing
//! produces request/import/export tables whose local binding indices point at
//! authenticated closure slots on the module root function.  Runtime
//! publication consumes the complete draft transactionally.

use crate::function::UnlinkedFunction;
use crate::value::JsString;

/// Rust spelling of QuickJS's private `JS_ATOM__default_` module binding.
/// Source text cannot name this cell.
pub(crate) const MODULE_DEFAULT_BINDING_NAME: &str = "<module-default>";

/// Unspellable compiler/runtime cell which caches one module's `import.meta`.
pub(crate) const MODULE_IMPORT_META_BINDING_NAME: &str = "<import.meta>";

/// Source-order index into [`UnlinkedModule::requested_modules`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ModuleRequestIndex(pub(crate) u32);

/// One decoded entry from a static import declaration's `with` clause.
///
/// QuickJS builds the corresponding null-prototype object in source order.
/// Keeping the ordered entries structural here preserves that observable
/// enumeration order without introducing a JavaScript heap object during
/// parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleImportAttribute {
    /// Decoded property key, after identifier and string-literal escapes.
    pub key: JsString,
    /// Decoded StringLiteral value.
    pub value: JsString,
}

/// Syntactic import-attribute state for one requested module.
///
/// `Present([])` deliberately differs from [`Self::Absent`] so compiler and
/// source tooling can retain whether `with {}` was authored. Pinned QuickJS
/// does not allocate an attributes object until the first entry, however, so
/// both states collapse to `None` through [`Self::effective`] at the host
/// checker and loader boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModuleImportAttributes {
    /// No `with` clause was authored.
    Absent,
    /// An authored `with` clause, possibly empty.
    Present(Box<[ModuleImportAttribute]>),
}

impl ModuleImportAttributes {
    /// Attributes exposed to QuickJS-compatible host hooks.
    #[must_use]
    pub fn effective(&self) -> Option<&[ModuleImportAttribute]> {
        match self {
            Self::Present(attributes) if !attributes.is_empty() => Some(attributes),
            Self::Absent | Self::Present(_) => None,
        }
    }

    /// Entries from an authored `with` clause, including an empty clause.
    #[must_use]
    pub fn syntactic(&self) -> Option<&[ModuleImportAttribute]> {
        match self {
            Self::Absent => None,
            Self::Present(attributes) => Some(attributes),
        }
    }
}

/// One normalized-later module specifier from an import or re-export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ModuleRequest {
    pub(crate) specifier: JsString,
    pub(crate) attributes: ModuleImportAttributes,
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
    Function {
        constant: u32,
        /// Optional root String constant consumed by QuickJS `OP_set_name`
        /// between closure creation and the declaration-cell write. This is
        /// present only for an anonymous default function declaration.
        inferred_name: Option<u32>,
    },
}

/// One exact link-entry write to a non-lexical module declaration cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModuleLinkInitializer {
    pub(crate) closure_index: u16,
    pub(crate) value: ModuleLinkInitializerValue,
}

/// One pinned QuickJS import/declaration name collision.
///
/// QuickJS resolves both records to the import's first closure slot. Ordinary
/// writes remain read-only, while declaration initialization has a narrowly
/// authenticated raw-write path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModuleImportCollision {
    pub(crate) closure_index: u16,
    pub(crate) declaration: ModuleImportCollisionDeclaration,
}

/// Declaration flavor sharing an import's effective closure slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleImportCollisionDeclaration {
    Var,
    Lexical,
    Function,
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
    declaration_order: Box<[u16]>,
    link_initializers: Box<[ModuleLinkInitializer]>,
    import_collisions: Box<[ModuleImportCollision]>,
    requested_modules: Box<[ModuleRequest]>,
    imports: Box<[ModuleImport]>,
    exports: Box<[ModuleExport]>,
    star_exports: Box<[ModuleStarExport]>,
}

/// Compiler-owned module tables published as one sealed aggregate.
pub(crate) struct UnlinkedModuleTables {
    pub(crate) declaration_order: Vec<u16>,
    pub(crate) link_initializers: Vec<ModuleLinkInitializer>,
    pub(crate) import_collisions: Vec<ModuleImportCollision>,
    pub(crate) requested_modules: Vec<ModuleRequest>,
    pub(crate) imports: Vec<ModuleImport>,
    pub(crate) exports: Vec<ModuleExport>,
    pub(crate) star_exports: Vec<ModuleStarExport>,
}

/// Owned pieces crossing the one-way module publication boundary.
pub(crate) struct UnlinkedModuleParts {
    pub(crate) name: JsString,
    pub(crate) function: UnlinkedFunction,
    pub(crate) declaration_order: Box<[u16]>,
    pub(crate) link_initializers: Box<[ModuleLinkInitializer]>,
    pub(crate) import_collisions: Box<[ModuleImportCollision]>,
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
        tables: UnlinkedModuleTables,
    ) -> Self {
        let UnlinkedModuleTables {
            declaration_order,
            link_initializers,
            import_collisions,
            requested_modules,
            imports,
            exports,
            star_exports,
        } = tables;
        Self {
            name,
            function,
            declaration_order: declaration_order.into_boxed_slice(),
            link_initializers: link_initializers.into_boxed_slice(),
            import_collisions: import_collisions.into_boxed_slice(),
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
    pub(crate) const fn declaration_order(&self) -> &[u16] {
        &self.declaration_order
    }

    #[must_use]
    pub(crate) const fn link_initializers(&self) -> &[ModuleLinkInitializer] {
        &self.link_initializers
    }

    #[must_use]
    pub(crate) const fn import_collisions(&self) -> &[ModuleImportCollision] {
        &self.import_collisions
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
            declaration_order: self.declaration_order,
            link_initializers: self.link_initializers,
            import_collisions: self.import_collisions,
            requested_modules: self.requested_modules,
            imports: self.imports,
            exports: self.exports,
            star_exports: self.star_exports,
        }
    }
}

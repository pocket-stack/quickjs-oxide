use super::*;

/// Constant-pool entry owned by a [`FunctionBytecodeData`] node.
#[derive(Clone, Debug, PartialEq)]
pub enum BytecodeConstant {
    Value(RawValue),
    /// Compile-once RegExp literal payload. These reference-counted Rust
    /// leaves own no arena or atom edge; executing `Instruction::RegExp`
    /// clones them into a fresh realm-local RegExp object.
    RegExp {
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    },
    Function(FunctionBytecodeId),
}

/// Publication-authenticated role of one class-private lexical capability.
///
/// Setter primary and synthetic `<set>` cells deliberately share
/// [`ClosureVariableKind::PrivateSetter`]. The runtime publisher is the only
/// layer which still owns both the exact source spelling and its interned
/// [`Atom`], so it seals that distinction here before handing linked bytecode
/// to the atom-table-independent heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublishedPrivateBindingRole {
    Primary,
    SetterStorage,
}

/// One non-owning identity authenticated by the runtime publisher. `name`
/// must equal the atom already owned by the corresponding variable definition
/// or closure descriptor. Local setter halves also carry reciprocal `pair`
/// indices; closure captures may legitimately retain only one half and leave
/// it absent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublishedPrivateBinding {
    pub(in crate::engine::heap) name: Atom,
    pub(in crate::engine::heap) role: PublishedPrivateBindingRole,
    pub(in crate::engine::heap) pair: Option<u16>,
}

impl PublishedPrivateBinding {
    #[must_use]
    pub(crate) const fn primary(name: Atom, pair: Option<u16>) -> Self {
        Self {
            name,
            role: PublishedPrivateBindingRole::Primary,
            pair,
        }
    }

    #[must_use]
    pub(crate) const fn setter_storage(name: Atom, pair: Option<u16>) -> Self {
        Self {
            name,
            role: PublishedPrivateBindingRole::SetterStorage,
            pair,
        }
    }
}

#[derive(Debug)]
pub(in crate::engine::heap) struct AuthenticatedPrivateBindings {
    pub(in crate::engine::heap) locals: Box<[Option<PublishedPrivateBinding>]>,
    pub(in crate::engine::heap) closures: Box<[Option<PublishedPrivateBinding>]>,
}

/// Sealed bridge between name-aware bytecode publication and the heap.
///
/// Public linked-bytecode callers can construct only [`Self::none`]. Any
/// private definition or closure descriptor requires the crate-internal
/// authenticated constructor, preventing a forged `FunctionBytecodeData`
/// from choosing whether a `PrivateSetter` cell is a primary name or its
/// synthetic write capability.
#[derive(Debug)]
pub struct PublishedPrivateBindings {
    pub(in crate::engine::heap) authenticated: Option<AuthenticatedPrivateBindings>,
}

impl PublishedPrivateBindings {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            authenticated: None,
        }
    }

    #[must_use]
    pub(crate) fn authenticated(
        locals: Vec<Option<PublishedPrivateBinding>>,
        closures: Vec<Option<PublishedPrivateBinding>>,
    ) -> Self {
        Self {
            authenticated: Some(AuthenticatedPrivateBindings {
                locals: locals.into_boxed_slice(),
                closures: closures.into_boxed_slice(),
            }),
        }
    }
}

impl Default for PublishedPrivateBindings {
    fn default() -> Self {
        Self::none()
    }
}

/// Runtime-owned immutable bytecode, constant pool, and function realm.
///
/// `code` is an `Rc` leaf with no runtime edges.  The VM may cheaply clone it
/// before dropping the runtime's `RefCell` borrow, while a bytecode root keeps
/// the raw constant pool alive for the duration of execution.
#[derive(Debug)]
pub struct FunctionBytecodeData {
    pub code: Rc<[Instruction]>,
    pub constants: Rc<[BytecodeConstant]>,
    pub realm: ContextId,
    pub metadata: FunctionMetadata,
    pub parameter_environment: Option<ParameterEnvironmentLayout>,
    /// Intrinsic source-level name. Contextual `SetName` inference remains a
    /// separate opcode and is only emitted for anonymous definitions.
    pub func_name: Option<JsString>,
    pub argument_definitions: Rc<[VariableDefinition]>,
    pub local_definitions: Rc<[VariableDefinition]>,
    pub closure_variables: Rc<[ClosureVariable]>,
    /// Name-bound private capability roles sealed by the runtime publisher.
    /// This metadata owns no atoms; identities alias definition/descriptor
    /// atoms whose references remain in `auxiliary_atoms`.
    pub private_bindings: PublishedPrivateBindings,
    pub eval_environments: Rc<[EvalEnvironment<Atom>]>,
    pub debug: Option<FunctionDebugInfo>,
    /// Atom references owned by bytecode metadata/opcode operands.
    ///
    /// Symbol constants are excluded: every `RawValue::Symbol` occurrence owns
    /// and releases its own separate atom reference.
    pub auxiliary_atoms: Box<[Atom]>,
}

/// Runtime-owned debug metadata for one bytecode function.
///
/// `filename` is backed by one distinct reference in `auxiliary_atoms`; it is
/// intentionally not released separately when the bytecode node dies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDebugInfo {
    pub filename: Atom,
    pub pc2line: Option<Pc2LineTable>,
    pub source: Option<Box<[u8]>>,
}

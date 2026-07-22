use crate::Error;
use crate::value::Value;
use std::collections::VecDeque;

/// QuickJS `JS_STACK_SIZE_MAX` for one bytecode function.
pub const MAX_STACK_SIZE: u16 = 65_534;
/// QuickJS `JS_MAX_LOCAL_VARS` for one bytecode function.
pub const MAX_LOCAL_SLOTS: u16 = 65_534;

/// Ordinary-function arguments object selected by QuickJS's
/// `OP_special_object` lowering.
///
/// Simple sloppy parameter lists share their argument cells with indexed
/// properties; strict (and, once supported, non-simple) functions receive an
/// independent snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArgumentsKind {
    Mapped,
    Unmapped,
}

/// Storage which owns one hidden sloppy-eval variable object.
///
/// The object itself lives in a compiler-authenticated local or closure
/// `VarRef`. Keeping that source typed prevents the dynamic-name opcodes from
/// accepting an arbitrary JavaScript-visible object from the operand stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EvalVariableSource {
    Local(u16),
    Closure(u16),
}

/// Storage which owns one hidden sloppy-`with` object.
///
/// Like [`EvalVariableSource`], this is compiler-authenticated frame state and
/// cannot be supplied by JavaScript through the operand stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WithObjectSource {
    Local(u16),
    Closure(u16),
}

/// One ordered object-environment record consulted by dynamic name lookup.
///
/// Keeping eval-variable and `with` sources distinct lets the host apply the
/// latter's `Symbol.unscopables` rules without weakening either source type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DynamicEnvironmentSource {
    Eval(EvalVariableSource),
    With(WithObjectSource),
}

const fn dynamic_environment_local(source: DynamicEnvironmentSource) -> Option<u16> {
    match source {
        DynamicEnvironmentSource::Eval(EvalVariableSource::Local(index))
        | DynamicEnvironmentSource::With(WithObjectSource::Local(index)) => Some(index),
        DynamicEnvironmentSource::Eval(EvalVariableSource::Closure(_))
        | DynamicEnvironmentSource::With(WithObjectSource::Closure(_)) => None,
    }
}

/// Object-literal function role carried by QuickJS's `OP_define_method`
/// family.
///
/// The role is deliberately part of the verified instruction rather than a
/// JavaScript stack value: it selects both the function-name prefix and the
/// property descriptor shape used at publication time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefineMethodKind {
    Method,
    Getter,
    Setter,
}

/// Typed form of QuickJS's `OP_apply` magic operand.
///
/// Both forms consume `function, this-or-new-target, argument-array`; the
/// distinction is authenticated bytecode metadata rather than a
/// JavaScript-visible stack value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplyKind {
    Call,
    Construct,
}

/// Stack-machine operations deliberately use the names and stack behavior of
/// their `QuickJS` counterparts. This typed form is the current compiler IR and
/// verified execution format; a future compact encoder must share this opcode
/// metadata instead of defining a second instruction contract.
#[derive(Clone, Debug)]
pub enum Instruction {
    Nop,
    PushI32(i32),
    PushConst(u32),
    FClosure(u32),
    /// QuickJS `OP_regexp`, represented as one typed compile-time constant
    /// instead of two generic stack values. Each execution allocates a fresh
    /// branded RegExp in the bytecode realm without consulting user code.
    RegExp(u32),
    /// QuickJS `OP_set_name`: conditionally define the name of the object at
    /// the top of the stack from a string constant, without consuming it.
    SetName(u32),
    /// QuickJS `OP_throw_error atom JS_THROW_VAR_RO` for a strict immutable
    /// function-expression name. Consumes the attempted value and terminates.
    ThrowReadOnly(u32),
    /// QuickJS `OP_throw_error atom JS_THROW_VAR_REDECL`: throw the eval-time
    /// SyntaxError used for a `var`/function declaration which crosses an
    /// imported lexical scope. The verified String constant preserves the
    /// conflicting binding identity even though QuickJS's message is generic.
    ThrowRedeclaration(u32),
    /// QuickJS `JS_THROW_ERROR_DELETE_SUPER`. The three authenticated
    /// SuperProperty Reference operands are consumed conceptually and the
    /// terminal throw occupies the expression's one-value stack shape.
    ThrowDeleteSuper,
    Undefined,
    Null,
    PushFalse,
    PushTrue,
    PushThis,
    /// Push the exact function object of the active bytecode frame. Derived
    /// constructors use its live [[Prototype]] as the target of `super()`, so
    /// this must not be reconstructed from HomeObject or the original heritage
    /// expression.
    PushActiveFunction,
    /// QuickJS's authenticated `<home_object>` pseudo binding. Only bytecode
    /// whose immutable metadata declares `needs_home_object` may execute this
    /// operation; the runtime reads it from the active function object rather
    /// than from a JavaScript-visible operand.
    PushHomeObject,
    PushNewTarget,
    /// QuickJS `OP_special_object` for an ordinary function's lazily selected
    /// arguments binding. Runtime creation still occurs in the entry prologue,
    /// before body function hoists.
    Arguments(ArgumentsKind),
    /// QuickJS `OP_rest`: collect the actual arguments beginning at this
    /// formal-parameter index into a fresh Array in the callee realm. The
    /// operand may equal `argument_count` for a rest BindingPattern, which
    /// owns no physical argument slot.
    Rest(u16),
    /// QuickJS `OP_special_object` with `OP_SPECIAL_OBJECT_VAR_OBJECT`: create
    /// the null-prototype object which stores names introduced by sloppy
    /// direct eval in this activation.
    VariableEnvironment,
    /// Test whether the hidden variable object has an own property named by a
    /// verified String constant.
    HasEvalVariable {
        source: EvalVariableSource,
        name: u32,
    },
    /// Read one property from the hidden variable object.
    GetEvalVariable {
        source: EvalVariableSource,
        name: u32,
    },
    /// Consume and assign one property on the hidden variable object.
    PutEvalVariable {
        source: EvalVariableSource,
        name: u32,
    },
    /// Delete one configurable property from the hidden variable object and
    /// push the Boolean result.
    DeleteEvalVariable {
        source: EvalVariableSource,
        name: u32,
    },
    /// Consume a value and define an own configurable, writable, enumerable
    /// data property on the hidden variable object.
    DefineEvalVariable {
        source: EvalVariableSource,
        name: u32,
    },
    /// ECMAScript `ToObject`: preserve Objects, reject nullish values, and box
    /// all other primitives in the executing realm.
    ToObject,
    /// Test whether an ordered hidden environment exposes one dynamic name.
    HasDynamicBinding {
        source: DynamicEnvironmentSource,
        name: u32,
    },
    /// Read one dynamic binding using the current frame's strictness.
    GetDynamicBinding {
        source: DynamicEnvironmentSource,
        name: u32,
    },
    /// Consume and write one dynamic binding using the current frame's
    /// strictness.
    PutDynamicBinding {
        source: DynamicEnvironmentSource,
        name: u32,
    },
    /// Delete one dynamic binding and push the Boolean result.
    DeleteDynamicBinding {
        source: DynamicEnvironmentSource,
        name: u32,
    },
    /// Push the authenticated object behind one dynamic environment source.
    DynamicEnvironmentObject(DynamicEnvironmentSource),
    /// QuickJS `OP_make_var_ref`: resolve one authenticated global closure
    /// name to the current realm's lexical storage object, global object, or
    /// the unresolved-reference `undefined` sentinel. Lexical TDZ and
    /// read-only checks happen while this reference is made, before a later
    /// right-hand side can run.
    GlobalReference(u16),
    /// Preserve an environment Object and append its named reference value.
    /// A missing property observes the current frame's strictness.
    GetRefValue(u32),
    /// Preserve an environment Object and append its named reference value,
    /// producing `undefined` when the property is missing.
    GetRefValueUndef(u32),
    /// Consume an environment Object and value and write the named reference
    /// using the current frame's strictness.
    PutRefValue(u32),
    GetLocal(u16),
    PutLocal(u16),
    SetLocal(u16),
    /// QuickJS `OP_set_loc_uninitialized`: enter one lexical scope with its
    /// local slot in the temporal dead zone.
    SetLocalUninitialized(u16),
    /// QuickJS `OP_get_loc_check`: read a lexical local after its TDZ check.
    GetLocalCheck(u16),
    /// Typed-format distinction for QuickJS's lexical-initializer `put_loc`.
    /// The published vardef must be lexical, but execution deliberately keeps
    /// ordinary `put_loc` overwrite semantics rather than conflating this with
    /// derived-`this`'s upstream `put_loc_check_init` opcode.
    InitializeLocal(u16),
    /// QuickJS `OP_put_loc_check_init`: consume the result of `super()` and
    /// initialize the authenticated derived-constructor `this` local exactly
    /// once. A second execution throws after the super constructor has run.
    InitializeDerivedLocal(u16),
    /// QuickJS `OP_put_loc_check`: consume and assign a mutable lexical local.
    PutLocalCheck(u16),
    /// QuickJS `OP_set_loc_check`: assign a mutable lexical local while
    /// preserving the assigned value on the operand stack.
    SetLocalCheck(u16),
    GetArg(u16),
    PutArg(u16),
    SetArg(u16),
    GetVarRef(u16),
    PutVarRef(u16),
    SetVarRef(u16),
    /// QuickJS `OP_get_var_ref_check`: read a captured lexical binding after
    /// its TDZ check.
    GetVarRefCheck(u16),
    /// QuickJS `OP_put_var_ref_check`: consume and assign a captured mutable
    /// lexical binding. Value-preserving writes use `Dup; PutVarRefCheck`.
    PutVarRefCheck(u16),
    /// Captured-binding counterpart of [`Instruction::InitializeDerivedLocal`].
    /// Arrow functions and direct eval use this to initialize their enclosing
    /// derived constructor's single `this` cell.
    InitializeDerivedVarRef(u16),
    /// QuickJS `OP_close_loc`: detach a captured local when its lexical scope
    /// exits so a later scope entry receives a fresh cell.
    CloseLocal(u16),
    /// QuickJS `OP_get_var`: read a global-environment VarRef closure slot.
    GetVar(u16),
    /// QuickJS `OP_get_var_undef`: as `GetVar`, but suppress a genuinely
    /// missing global binding for a direct `typeof IdentifierReference`.
    GetVarUndef(u16),
    /// QuickJS `OP_delete_var`: delete one sloppy direct global binding and
    /// push the Boolean result. Lexical and resolved non-global bindings are
    /// folded to `PushFalse` during scope resolution.
    DeleteVar(u16),
    /// QuickJS `OP_put_var`: assign and consume a global binding value.
    PutVar(u16),
    /// QuickJS `OP_put_var_init`: initialize a lexical global binding.
    PutVarInit(u16),
    /// QuickJS `OP_get_field`: replace an object-like base with the value of
    /// one constant string-keyed property.
    GetField(u32),
    /// QuickJS `OP_get_field2`: keep the base below the fetched value so a
    /// following `CallMethod` observes the original reference receiver.
    GetField2(u32),
    /// QuickJS `OP_get_array_el`: `base key -> value`, including observable
    /// `ToPropertyKey` conversion in the runtime host.
    GetArrayEl,
    /// QuickJS `OP_get_array_el2`: `base key -> base value` for method calls.
    GetArrayEl2,
    /// QuickJS `OP_get_array_el3`: `base raw-key -> base converted-key value`
    /// for compound/logical assignment without repeated key conversion.
    GetArrayEl3,
    /// QuickJS `OP_get_super`: replace a method HomeObject with its current
    /// prototype. A null prototype is represented by JavaScript `null` and is
    /// diagnosed only by the following property operation.
    GetSuper,
    /// QuickJS `OP_get_super_value`: `receiver base key -> value`. The base is
    /// the prototype captured before a computed key is evaluated, while
    /// accessors observe the method's actual `this` as their receiver.
    GetSuperValue,
    /// QuickJS's call-site rewrite of `OP_get_super_value` to
    /// `OP_get_array_el`: `receiver base key -> receiver value`. This
    /// deliberately gives a getter `base` as its receiver before the returned
    /// callable is invoked with the preserved method receiver.
    GetSuperValueForCall,
    /// QuickJS `OP_array_from`: consume `element_count` dense literal values
    /// and replace them with a fresh Array in the bytecode's realm.
    ArrayFrom(u16),
    /// QuickJS `OP_object`: push a fresh ordinary Object rooted in the
    /// executing bytecode's realm.
    Object,
    /// QuickJS `OP_to_propkey`: observable `ToPropertyKey` while retaining a
    /// canonical Int/String/Symbol value on the VM stack.
    ToPropKey,
    /// QuickJS `OP_insert2`: `base value -> value base value`.
    Insert2,
    /// QuickJS `OP_insert3`: `base key value -> value base key value`.
    Insert3,
    /// QuickJS `OP_dup3`: `a b c -> a b c a b c`.
    Dup3,
    /// QuickJS `OP_insert4`: `this base key value -> value this base key value`.
    Insert4,
    /// QuickJS `OP_perm3`: `base old new -> old base new`.
    Perm3,
    /// QuickJS `OP_perm4`: `base key old new -> old base key new`.
    Perm4,
    /// QuickJS `OP_perm5`: `this base key old new -> old this base key new`.
    Perm5,
    /// QuickJS `OP_rot4l`: `value this base key -> this base key value`.
    Rot4Left,
    /// QuickJS `OP_put_field`: assign one constant string-keyed property.
    PutField(u32),
    /// QuickJS `OP_put_array_el`: assign a computed property, converting the
    /// still-raw key only after the right-hand side has been evaluated.
    PutArrayEl,
    /// QuickJS `OP_put_super_value`: assign through `base` with `receiver` as
    /// the actual receiver. `JS_PROP_THROW_STRICT` rejects failed writes only
    /// when the containing method is strict.
    PutSuperValue,
    /// QuickJS `OP_define_field`: define one C_W_E data property while
    /// preserving the object below it (`object value -> object`). Array
    /// literals use this after their initial dense prefix.
    DefineField(u32),
    /// Computed-key class-field form of QuickJS `OP_define_array_el`. The key
    /// is the authenticated output of `ToPropKey`, so execution must not run
    /// observable property-key conversion again. Defines one C_W_E own data
    /// property while preserving only the object
    /// (`object canonical-key value -> object`).
    DefineFieldComputed,
    /// QuickJS `OP_define_method`: name and publish an object-literal method
    /// under one verified String constant while preserving the fresh literal
    /// (`object closure -> object`).
    DefineMethod {
        key: u32,
        kind: DefineMethodKind,
        enumerable: bool,
    },
    /// QuickJS `OP_define_method_computed`: publish an object-literal method
    /// under an already-canonical property key while preserving the fresh
    /// literal (`object key closure -> object`).
    DefineMethodComputed {
        kind: DefineMethodKind,
        enumerable: bool,
    },
    /// QuickJS `OP_define_class`: replace `parent, constructor` with the
    /// published class `constructor, prototype` pair. The heritage flag is
    /// authenticated bytecode metadata rather than a JavaScript stack value.
    DefineClass {
        name: u32,
        has_heritage: bool,
    },
    /// Attach one authenticated hidden instance-fields program to a freshly
    /// defined class constructor while installing the class prototype as its
    /// HomeObject (`constructor prototype initializer -> constructor prototype`).
    InstallClassInstanceInitializer,
    /// Run the hidden instance-fields program attached to the active class
    /// constructor with the retained receiver as `this`
    /// (`receiver active-constructor -> receiver`).
    /// Absence of an initializer is the ordinary no-fields fast path.
    CallClassInstanceInitializer,
    /// Install the constructor as HomeObject and immediately execute the
    /// aggregate static-elements program (`constructor initializer -> constructor`).
    RunClassStaticInitializer,
    /// Execute a non-escaping static-block child with the aggregate static
    /// initializer's receiver and HomeObject (`block ->`).
    CallClassStaticBlock,
    /// QuickJS `OP_define_array_el`: define a computed C_W_E data property
    /// while preserving the Array and its dynamic index
    /// (`array index value -> array index`).
    DefineArrayEl,
    /// QuickJS `OP_set_name_computed`: infer an anonymous function's name
    /// from an already-canonical computed property key while preserving both
    /// operands (`key value -> key value`).
    SetNameComputed,
    /// QuickJS `OP_set_proto`: consume one object-literal prototype candidate
    /// and preserve the fresh literal (`object value -> object`). Object and
    /// null candidates replace [[Prototype]]; primitive candidates are ignored.
    SetProto,
    /// Object-literal specialization of QuickJS `OP_copy_data_properties`.
    /// It snapshots enumerable own String/Symbol keys from an Object source,
    /// defines C_W_E data properties, and preserves the fresh literal
    /// (`target source -> target`).
    CopyDataProperties,
    /// Exclusion-aware QuickJS `OP_copy_data_properties` used by an Object
    /// binding rest element. The three operands are addressed by depth from
    /// the top of the stack (depth zero is the top) and are only borrowed:
    /// successful execution leaves the complete operand stack unchanged.
    ///
    /// This is intentionally depth-addressed instead of consuming three
    /// adjacent values. A sloppy `var` binding may need to keep its prepared
    /// Reference between the retained source and the fresh target while the
    /// copy executes observable getters.
    CopyDataPropertiesExcluded {
        target_depth: u8,
        source_depth: u8,
        excluded_depth: u8,
    },
    /// QuickJS `OP_append`: append every value from one iterable and replace
    /// the dynamic index with the first unused index
    /// (`array index iterable -> array index`).
    Append,
    /// QuickJS `OP_delete`: `base key -> bool` with strictness supplied by the
    /// active call frame.
    Delete,
    Drop,
    /// QuickJS `OP_nip`: discard the value immediately below the stack top,
    /// preserving the top value (`a b -> b`).
    Nip,
    /// QuickJS `OP_swap`: exchange the top two operands (`a b -> b a`).
    Swap,
    Dup,
    /// QuickJS `OP_dup1`: duplicate the value immediately below the stack top
    /// in place (`a b -> a a b`).
    Dup1,
    Neg,
    Plus,
    Inc,
    Dec,
    /// QuickJS `OP_post_inc`: convert the operand to numeric, then leave the
    /// old numeric value below its incremented replacement.
    PostInc,
    /// QuickJS `OP_post_dec`: convert the operand to numeric, then leave the
    /// old numeric value below its decremented replacement.
    PostDec,
    BitNot,
    Not,
    TypeOf,
    IsUndefinedOrNull,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Sar,
    Shr,
    BitAnd,
    BitXor,
    BitOr,
    Eq,
    StrictEq,
    Neq,
    StrictNeq,
    Lt,
    Lte,
    Gt,
    Gte,
    InstanceOf,
    In,
    IfFalse(u32),
    IfTrue(u32),
    Goto(u32),
    /// QuickJS `OP_catch`, represented as private VM handler metadata rather
    /// than a forgeable operand-stack value.
    Catch(u32),
    /// Remove the innermost active catch marker on a normal path.
    DropCatch,
    /// Preserve the top value while discarding the innermost catch marker and
    /// every intermediate operand above its entry depth.
    NipCatch,
    /// QuickJS `OP_gosub`: push the fallthrough PC as a real Int value and
    /// enter a shared finally subroutine.
    Gosub(u32),
    /// QuickJS `OP_ret`: pop and validate the Int return PC from `Gosub`.
    Ret,
    /// Typed finally cleanup for an abrupt path which discards, rather than
    /// follows, the real Int return PC pushed by `Gosub`.
    DropGosub,
    /// QuickJS `OP_for_of_start`: replace one iterable with the two public
    /// iterator-record values (`iterator`, `next`) plus a private unwind
    /// marker represented only in the verifier and VM region stack.
    ForOfStart,
    /// QuickJS `OP_for_of_next`: retain the three-slot iterator record below
    /// `offset` intermediate operands and append `value`, `done`.
    ForOfNext(u8),
    /// QuickJS `OP_for_in_start`: replace one source value with a hidden
    /// enumeration object.
    ForInStart,
    /// QuickJS `OP_for_in_next`: retain the enumeration object and append the
    /// next string key plus its `done` flag.
    ForInNext,
    /// QuickJS `OP_iterator_close`: remove the innermost iterator record and
    /// close it for a normal abrupt completion.
    IteratorClose,
    /// Preserve the top operand while removing the innermost iterator record
    /// and every intermediate operand, then close the iterator. This is the
    /// typed equivalent of QuickJS's `nip_catch; rot3r; undefined;
    /// iterator_close` return-unwind sequence.
    IteratorClosePreserve,
    Call(u16),
    /// QuickJS `OP_eval`: a syntactic direct-eval call site. The runtime first
    /// compares the resolved callee with the executing realm's original
    /// `%eval%`; a replacement callee falls back to an ordinary call with an
    /// undefined receiver. `environment` selects the immutable linked lexical
    /// environment published beside this function's bytecode; String-source
    /// execution retains it for authenticated nested direct-eval relays.
    Eval {
        argument_count: u16,
        environment: u16,
    },
    CallMethod(u16),
    /// QuickJS `OP_call_constructor`: `func new.target args -> result`.
    Construct(u16),
    /// Authenticate the two stack operands prepared for a derived-constructor
    /// `super()` call. Runtime execution is a no-op; the bytecode verifier
    /// protects the marked `super_constructor, new.target` pair until exactly
    /// one matching typed construction consumes it.
    MarkSuperCall,
    /// Derived-constructor counterpart of [`Instruction::Construct`]. The
    /// verifier accepts it only when it exactly consumes the most recent
    /// protected super-call pair together with `argument_count` values.
    ConstructSuper(u16),
    /// QuickJS `OP_check_ctor`: reject an ordinary call to a class
    /// constructor by checking the current frame's `new.target`.
    CheckCtor,
    /// QuickJS `OP_init_ctor`: run the implicit derived constructor by reading
    /// the active function object's live [[Prototype]], forwarding the frame's
    /// raw actual arguments, and preserving the original `new.target`.
    InitDerivedConstructor,
    /// QuickJS `OP_apply`: consume a compiler-created dense argument Array
    /// after all spread iterables have been evaluated.
    Apply(ApplyKind),
    /// Spread-argument counterpart of [`Instruction::ConstructSuper`].
    ApplySuper,
    /// QuickJS `OP_apply_eval`: the spread-argument counterpart of `Eval`.
    /// The original-eval identity check happens only after the dense argument
    /// Array has passed through QuickJS-compatible `build_arg_list` semantics.
    ApplyEval {
        environment: u16,
    },
    Return,
    /// Complete a derived constructor. Object results are returned directly;
    /// `undefined` resolves the authenticated derived-`this` local; every other
    /// primitive throws a TypeError. Return-protocol errors belong to the
    /// caller realm rather than the constructor's defining realm.
    ReturnDerived(u16),
    Throw,
}

impl Instruction {
    #[must_use]
    pub const fn stack_effect(&self) -> (usize, usize) {
        match self {
            Self::Nop
            | Self::CheckCtor
            | Self::Goto(_)
            | Self::Gosub(_)
            | Self::ThrowRedeclaration(_)
            | Self::SetLocalUninitialized(_)
            | Self::CloseLocal(_) => (0, 0),
            // The verifier models this marker in its conceptual stack even
            // though runtime execution stores it in private handler metadata.
            Self::Catch(_) => (0, 1),
            Self::ForOfStart => (1, 3),
            Self::ForOfNext(_) => (3, 5),
            Self::ForInStart => (1, 1),
            Self::ForInNext => (1, 3),
            Self::PushI32(_)
            | Self::PushConst(_)
            | Self::FClosure(_)
            | Self::RegExp(_)
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::InitDerivedConstructor
            | Self::Arguments(_)
            | Self::Rest(_)
            | Self::VariableEnvironment
            | Self::HasEvalVariable { .. }
            | Self::GetEvalVariable { .. }
            | Self::DeleteEvalVariable { .. }
            | Self::HasDynamicBinding { .. }
            | Self::GetDynamicBinding { .. }
            | Self::DeleteDynamicBinding { .. }
            | Self::DynamicEnvironmentObject(_)
            | Self::GlobalReference(_)
            | Self::GetLocal(_)
            | Self::GetLocalCheck(_)
            | Self::GetArg(_)
            | Self::GetVarRef(_)
            | Self::GetVarRefCheck(_)
            | Self::GetVar(_)
            | Self::GetVarUndef(_)
            | Self::DeleteVar(_) => (0, 1),
            Self::SetName(_) | Self::ToObject => (1, 1),
            Self::MarkSuperCall => (2, 2),
            Self::GetRefValue(_) | Self::GetRefValueUndef(_) => (1, 2),
            Self::GetField(_) => (1, 1),
            Self::GetField2(_) => (1, 2),
            Self::GetArrayEl => (2, 1),
            Self::GetArrayEl2 => (2, 2),
            Self::GetArrayEl3 => (2, 3),
            Self::GetSuper => (1, 1),
            Self::GetSuperValue => (3, 1),
            Self::GetSuperValueForCall => (3, 2),
            Self::ArrayFrom(element_count) => (*element_count as usize, 1),
            Self::Object => (0, 1),
            Self::ToPropKey => (1, 1),
            Self::Insert2 => (2, 3),
            Self::Insert3 => (3, 4),
            Self::Dup3 => (3, 6),
            Self::Insert4 => (4, 5),
            Self::Perm3 => (3, 3),
            Self::Perm4 => (4, 4),
            Self::Perm5 => (5, 5),
            Self::Rot4Left => (4, 4),
            Self::PutField(_) => (2, 0),
            Self::PutArrayEl => (3, 0),
            Self::PutSuperValue => (4, 0),
            Self::DefineField(_) | Self::DefineMethod { .. } => (2, 1),
            Self::DefineFieldComputed | Self::DefineMethodComputed { .. } => (3, 1),
            Self::DefineClass { .. } => (2, 2),
            Self::InstallClassInstanceInitializer => (3, 2),
            Self::CallClassInstanceInitializer => (2, 1),
            Self::RunClassStaticInitializer => (2, 1),
            Self::CallClassStaticBlock => (1, 0),
            Self::DefineArrayEl | Self::Append => (3, 2),
            Self::SetNameComputed => (2, 2),
            Self::SetProto | Self::CopyDataProperties => (2, 1),
            Self::CopyDataPropertiesExcluded {
                target_depth,
                source_depth,
                excluded_depth,
            } => {
                let mut maximum = *target_depth;
                if *source_depth > maximum {
                    maximum = *source_depth;
                }
                if *excluded_depth > maximum {
                    maximum = *excluded_depth;
                }
                let required = maximum as usize + 1;
                (required, required)
            }
            Self::Delete => (2, 1),
            Self::Call(argument_count) | Self::Eval { argument_count, .. } => {
                (*argument_count as usize + 1, 1)
            }
            Self::CallMethod(argument_count) => (*argument_count as usize + 2, 1),
            Self::Construct(argument_count) | Self::ConstructSuper(argument_count) => {
                (*argument_count as usize + 2, 1)
            }
            Self::Apply(_) | Self::ApplySuper => (3, 1),
            Self::ApplyEval { .. } => (2, 1),
            Self::Drop
            | Self::PutEvalVariable { .. }
            | Self::DefineEvalVariable { .. }
            | Self::PutDynamicBinding { .. }
            | Self::PutLocal(_)
            | Self::InitializeLocal(_)
            | Self::InitializeDerivedLocal(_)
            | Self::PutLocalCheck(_)
            | Self::PutArg(_)
            | Self::PutVarRef(_)
            | Self::PutVarRefCheck(_)
            | Self::InitializeDerivedVarRef(_)
            | Self::PutVar(_)
            | Self::PutVarInit(_)
            | Self::ThrowReadOnly(_)
            | Self::DropCatch
            | Self::DropGosub
            | Self::IfFalse(_)
            | Self::IfTrue(_)
            | Self::Return
            | Self::ReturnDerived(_)
            | Self::Throw => (1, 0),
            Self::PutRefValue(_) => (2, 0),
            Self::ThrowDeleteSuper => (3, 1),
            Self::Nip => (2, 1),
            Self::Swap => (2, 2),
            // The verifier replaces this nominal value-preserving effect with
            // the active handler's recorded entry depth.
            Self::NipCatch | Self::IteratorClosePreserve => (1, 1),
            Self::IteratorClose => (3, 0),
            Self::SetLocal(_) | Self::SetLocalCheck(_) | Self::SetArg(_) | Self::SetVarRef(_) => {
                (1, 1)
            }
            Self::Dup => (1, 2),
            Self::Dup1 => (2, 3),
            Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::BitNot
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull => (1, 1),
            Self::PostInc | Self::PostDec => (1, 2),
            Self::Add
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::Mod
            | Self::Pow
            | Self::Shl
            | Self::Sar
            | Self::Shr
            | Self::BitAnd
            | Self::BitXor
            | Self::BitOr
            | Self::Eq
            | Self::StrictEq
            | Self::Neq
            | Self::StrictNeq
            | Self::Lt
            | Self::Lte
            | Self::Gt
            | Self::Gte
            | Self::In
            | Self::InstanceOf => (2, 1),
            Self::Ret => (1, 0),
        }
    }

    /// Return the linked direct-eval environment carried by either fixed- or
    /// spread-argument eval bytecode.
    #[must_use]
    pub const fn eval_environment(&self) -> Option<u16> {
        match self {
            Self::Eval { environment, .. } | Self::ApplyEval { environment } => Some(*environment),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BytecodeFunction {
    pub name: Option<String>,
    pub code: Vec<Instruction>,
    pub constants: Vec<Value>,
    /// Declared local frame width. Local operands are verified against this
    /// boundary even when their instructions are unreachable.
    pub local_count: u16,
    pub max_stack: u16,
}

impl BytecodeFunction {
    #[must_use]
    pub fn constant(&self, index: u32) -> Option<&Value> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
    }

    /// Validate control flow, operands, and stack depth before execution.
    ///
    /// # Errors
    /// Returns an internal error when bytecode is malformed. Source compilation
    /// must never produce such an error; decoders must reject it before use.
    pub fn verify(&self) -> Result<VerifiedBytecode, Error> {
        if self.local_count > MAX_LOCAL_SLOTS {
            return Err(Error::internal(
                "declared local count exceeds QuickJS JS_MAX_LOCAL_VARS",
            ));
        }
        for instruction in &self.code {
            if matches!(instruction, Instruction::RegExp(_)) {
                // Detached bytecode exposes only `Vec<Value>` constants, so
                // it cannot structurally represent the compile-once matcher
                // payload required by this runtime-publication opcode.
                return Err(Error::internal(
                    "detached bytecode cannot encode RegExp constants",
                ));
            }
            if let Instruction::SetName(index)
            | Instruction::ThrowReadOnly(index)
            | Instruction::ThrowRedeclaration(index)
            | Instruction::GetField(index)
            | Instruction::GetField2(index)
            | Instruction::PutField(index)
            | Instruction::DefineField(index)
            | Instruction::DefineMethod { key: index, .. }
            | Instruction::DefineClass { name: index, .. }
            | Instruction::HasEvalVariable { name: index, .. }
            | Instruction::GetEvalVariable { name: index, .. }
            | Instruction::PutEvalVariable { name: index, .. }
            | Instruction::DeleteEvalVariable { name: index, .. }
            | Instruction::DefineEvalVariable { name: index, .. }
            | Instruction::HasDynamicBinding { name: index, .. }
            | Instruction::GetDynamicBinding { name: index, .. }
            | Instruction::PutDynamicBinding { name: index, .. }
            | Instruction::DeleteDynamicBinding { name: index, .. }
            | Instruction::GetRefValue(index)
            | Instruction::GetRefValueUndef(index)
            | Instruction::PutRefValue(index) = instruction
                && !matches!(self.constant(*index), Some(Value::String(_)))
            {
                return Err(Error::internal(
                    "string-key opcode referenced a non-string constant",
                ));
            }
            if let Instruction::GetLocal(index)
            | Instruction::PutLocal(index)
            | Instruction::SetLocal(index)
            | Instruction::SetLocalUninitialized(index)
            | Instruction::GetLocalCheck(index)
            | Instruction::InitializeLocal(index)
            | Instruction::InitializeDerivedLocal(index)
            | Instruction::PutLocalCheck(index)
            | Instruction::SetLocalCheck(index)
            | Instruction::CloseLocal(index)
            | Instruction::ReturnDerived(index) = instruction
                && *index >= self.local_count
            {
                return Err(Error::internal("local bytecode operand is out of bounds"));
            }
            if let Instruction::HasEvalVariable {
                source: EvalVariableSource::Local(index),
                ..
            }
            | Instruction::GetEvalVariable {
                source: EvalVariableSource::Local(index),
                ..
            }
            | Instruction::PutEvalVariable {
                source: EvalVariableSource::Local(index),
                ..
            }
            | Instruction::DeleteEvalVariable {
                source: EvalVariableSource::Local(index),
                ..
            }
            | Instruction::DefineEvalVariable {
                source: EvalVariableSource::Local(index),
                ..
            } = instruction
                && *index >= self.local_count
            {
                return Err(Error::internal(
                    "eval variable-object local operand is out of bounds",
                ));
            }
            let dynamic_source = match instruction {
                Instruction::HasDynamicBinding { source, .. }
                | Instruction::GetDynamicBinding { source, .. }
                | Instruction::PutDynamicBinding { source, .. }
                | Instruction::DeleteDynamicBinding { source, .. }
                | Instruction::DynamicEnvironmentObject(source) => Some(*source),
                _ => None,
            };
            if dynamic_source
                .and_then(dynamic_environment_local)
                .is_some_and(|index| index >= self.local_count)
            {
                return Err(Error::internal(
                    "dynamic environment local operand is out of bounds",
                ));
            }
        }
        verify_parts(&self.code, self.constants.len(), self.max_stack)
    }
}

/// Verify immutable bytecode parts before they enter the runtime heap.
///
/// Constant kinds are runtime-owned after linking, but control-flow validation
/// only needs the pool length. Keeping this verifier representation-neutral
/// lets publication validate child-function constants without manufacturing
/// temporary public [`Value`]s.
pub(crate) fn verify_parts(
    code: &[Instruction],
    constant_count: usize,
    declared_max_stack: u16,
) -> Result<VerifiedBytecode, Error> {
    if declared_max_stack > MAX_STACK_SIZE {
        return Err(Error::internal(
            "declared bytecode stack exceeds QuickJS JS_STACK_SIZE_MAX",
        ));
    }
    if code.is_empty() {
        return Err(Error::internal("bytecode function has no instructions"));
    }

    // Publication is a trust boundary. Validate representation operands for
    // every instruction, including dead code, before the reachability walk.
    // Later kind-specific verification may then index the constant pool
    // without letting malformed unreachable bytecode panic the runtime.
    for (pc, instruction) in code.iter().enumerate() {
        match instruction {
            Instruction::PushConst(index)
            | Instruction::FClosure(index)
            | Instruction::RegExp(index)
            | Instruction::SetName(index)
            | Instruction::ThrowReadOnly(index)
            | Instruction::ThrowRedeclaration(index)
            | Instruction::GetField(index)
            | Instruction::GetField2(index)
            | Instruction::PutField(index)
            | Instruction::DefineField(index)
            | Instruction::DefineMethod { key: index, .. }
            | Instruction::DefineClass { name: index, .. }
            | Instruction::HasEvalVariable { name: index, .. }
            | Instruction::GetEvalVariable { name: index, .. }
            | Instruction::PutEvalVariable { name: index, .. }
            | Instruction::DeleteEvalVariable { name: index, .. }
            | Instruction::DefineEvalVariable { name: index, .. }
            | Instruction::HasDynamicBinding { name: index, .. }
            | Instruction::GetDynamicBinding { name: index, .. }
            | Instruction::PutDynamicBinding { name: index, .. }
            | Instruction::DeleteDynamicBinding { name: index, .. }
            | Instruction::GetRefValue(index)
            | Instruction::GetRefValueUndef(index)
            | Instruction::PutRefValue(index) => {
                let is_valid = usize::try_from(*index)
                    .ok()
                    .is_some_and(|index| index < constant_count);
                if !is_valid {
                    return Err(Error::internal("constant index is out of bounds"));
                }
            }
            Instruction::Goto(target)
            | Instruction::IfFalse(target)
            | Instruction::IfTrue(target)
            | Instruction::Catch(target)
            | Instruction::Gosub(target) => {
                validate_target(*target, code.len())?;
                if matches!(instruction, Instruction::Gosub(_)) {
                    let return_pc = pc
                        .checked_add(1)
                        .ok_or_else(|| Error::internal("gosub return PC overflow"))?;
                    i32::try_from(return_pc)
                        .map_err(|_| Error::internal("gosub return PC does not fit Int"))?;
                }
            }
            _ => {}
        }
    }

    let mut states: Vec<Option<VerificationState>> = vec![None; code.len()];
    let mut worklist = VecDeque::from([(
        0_usize,
        VerificationState {
            depth: 0,
            regions: Vec::new(),
            return_addresses: Vec::new(),
            super_call_bases: Vec::new(),
        },
    )]);
    let mut maximum = 0_usize;

    while let Some((pc, state)) = worklist.pop_front() {
        record_maximum_depth(&mut maximum, state.depth, declared_max_stack)?;
        let slot = states
            .get_mut(pc)
            .ok_or_else(|| Error::internal("control flow target is out of bounds"))?;
        if let Some(previous) = slot {
            if previous != &state {
                let message = if previous.depth != state.depth {
                    "control flow joins with inconsistent stack depth"
                } else if previous.regions != state.regions {
                    "control flow joins with inconsistent unwind regions"
                } else if previous.return_addresses != state.return_addresses {
                    "control flow joins with inconsistent gosub return addresses"
                } else {
                    "control flow joins with inconsistent super-call markers"
                };
                return Err(Error::internal(message));
            }
            continue;
        }
        *slot = Some(state.clone());

        let instruction = &code[pc];
        let (popped, pushed) = instruction.stack_effect();
        let remaining_depth = state
            .depth
            .checked_sub(popped)
            .ok_or_else(|| Error::internal("bytecode stack underflow"))?;
        let mut next_depth = remaining_depth
            .checked_add(pushed)
            .ok_or_else(|| Error::internal("bytecode stack depth overflow"))?;
        let mut next_regions = state.regions.clone();
        let mut next_return_addresses = state.return_addresses.clone();
        let mut next_super_call_bases = state.super_call_bases.clone();

        if !matches!(
            instruction,
            Instruction::ConstructSuper(_) | Instruction::ApplySuper | Instruction::ForOfNext(_)
        ) {
            verify_super_call_pair_untouched(&state, remaining_depth, popped)?;
        }

        match instruction {
            Instruction::MarkSuperCall => {
                verify_ordinary_consumption(&state, remaining_depth, popped)?;
                next_super_call_bases.push(remaining_depth);
            }
            Instruction::ConstructSuper(_) | Instruction::ApplySuper => {
                if next_super_call_bases.last().copied() != Some(remaining_depth) {
                    return Err(Error::internal(
                        "typed super construction did not consume its protected pair",
                    ));
                }
                verify_ordinary_consumption(&state, remaining_depth, popped)?;
                next_super_call_bases.pop();
            }
            Instruction::DropCatch => {
                let region = next_regions
                    .pop()
                    .ok_or_else(|| Error::internal("DropCatch has no active catch handler"))?;
                let UnwindRegionState::Catch { marker_depth, .. } = region else {
                    return Err(Error::internal(
                        "DropCatch did not target the innermost unwind region",
                    ));
                };
                if state.depth != marker_depth {
                    return Err(Error::internal(
                        "DropCatch did not reach its catch entry depth",
                    ));
                }
            }
            Instruction::NipCatch => {
                let region = next_regions
                    .pop()
                    .ok_or_else(|| Error::internal("NipCatch has no active catch handler"))?;
                let UnwindRegionState::Catch { marker_depth, .. } = region else {
                    return Err(Error::internal(
                        "NipCatch did not target the innermost unwind region",
                    ));
                };
                if state.depth <= marker_depth {
                    return Err(Error::internal(
                        "NipCatch has no value above its catch marker",
                    ));
                }
                if state
                    .return_addresses
                    .last()
                    .is_some_and(|address| *address == state.depth - 1)
                {
                    return Err(Error::internal(
                        "NipCatch cannot preserve a gosub return address",
                    ));
                }
                let marker_index = marker_depth - 1;
                // QuickJS `OP_nip_catch` deliberately performs a dynamic
                // truncation: an abrupt completion from a nested finally may
                // discard the real return PC of a finally subroutine together
                // with every other value above the catch marker.  Keep an
                // address below the marker typed, reject preserving one as the
                // result above, and forget only addresses that the truncation
                // actually destroys.
                next_return_addresses.retain(|address| *address < marker_index);
                next_depth = marker_depth;
            }
            Instruction::ForOfStart => {
                verify_ordinary_consumption(&state, remaining_depth, popped)?;
                let record_base = remaining_depth;
                next_regions.push(UnwindRegionState::Iterator {
                    record_base,
                    marker_depth: next_depth,
                });
            }
            Instruction::ForOfNext(offset) => {
                let Some(UnwindRegionState::Iterator { marker_depth, .. }) = next_regions.last()
                else {
                    return Err(Error::internal(
                        "ForOfNext has no innermost iterator region",
                    ));
                };
                let expected_depth = marker_depth
                    .checked_add(usize::from(*offset))
                    .ok_or_else(|| Error::internal("for-of offset overflow"))?;
                if state.depth != expected_depth {
                    return Err(Error::internal(
                        "ForOfNext offset does not reach its iterator record",
                    ));
                }
                let record_base = marker_depth.checked_sub(3).ok_or_else(|| {
                    Error::internal("ForOfNext iterator marker has invalid depth")
                })?;
                if state
                    .super_call_bases
                    .iter()
                    .any(|base| *base < *marker_depth && base.saturating_add(2) > record_base)
                {
                    return Err(Error::internal(
                        "ForOfNext touched a protected super-call pair",
                    ));
                }
            }
            Instruction::IteratorClose => {
                let region = next_regions
                    .pop()
                    .ok_or_else(|| Error::internal("IteratorClose has no iterator region"))?;
                let UnwindRegionState::Iterator {
                    record_base,
                    marker_depth,
                } = region
                else {
                    return Err(Error::internal(
                        "IteratorClose did not target the innermost unwind region",
                    ));
                };
                if state.depth != marker_depth || remaining_depth != record_base {
                    return Err(Error::internal(
                        "IteratorClose did not reach its iterator record",
                    ));
                }
                if state
                    .return_addresses
                    .iter()
                    .any(|address| *address >= record_base)
                {
                    return Err(Error::internal(
                        "IteratorClose cannot discard a gosub return address",
                    ));
                }
            }
            Instruction::IteratorClosePreserve => {
                let region = next_regions.pop().ok_or_else(|| {
                    Error::internal("IteratorClosePreserve has no iterator region")
                })?;
                let UnwindRegionState::Iterator {
                    record_base,
                    marker_depth,
                } = region
                else {
                    return Err(Error::internal(
                        "IteratorClosePreserve did not target the innermost unwind region",
                    ));
                };
                if state.depth <= marker_depth {
                    return Err(Error::internal(
                        "IteratorClosePreserve has no value above its iterator marker",
                    ));
                }
                if state
                    .return_addresses
                    .last()
                    .is_some_and(|address| *address == state.depth - 1)
                {
                    return Err(Error::internal(
                        "IteratorClosePreserve cannot preserve a gosub return address",
                    ));
                }
                // Like `NipCatch`, this is a dynamic abrupt-completion
                // cleanup. A return which crosses nested finally and for-of
                // regions may have genuine Gosub return PCs among the
                // intermediate values that are intentionally truncated.
                // Addresses below the iterator record survive and remain
                // typed; the preserved top was rejected above if it was an
                // address itself.
                next_return_addresses.retain(|address| *address < record_base);
                next_depth = record_base
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("bytecode stack depth overflow"))?;
            }
            Instruction::Ret | Instruction::DropGosub => {
                if next_return_addresses.last().copied() != Some(state.depth - 1) {
                    let name = if matches!(instruction, Instruction::Ret) {
                        "Ret"
                    } else {
                        "DropGosub"
                    };
                    return Err(Error::internal(format!(
                        "{name} did not consume a gosub return address"
                    )));
                }
                next_return_addresses.pop();
                if state
                    .regions
                    .last()
                    .is_some_and(|region| remaining_depth < region.marker_depth())
                {
                    return Err(Error::internal(
                        "gosub return address crossed an active unwind marker",
                    ));
                }
            }
            _ => {
                verify_ordinary_consumption(&state, remaining_depth, popped)?;
            }
        }

        if next_super_call_bases
            .last()
            .is_some_and(|base| next_depth < base.saturating_add(2))
        {
            return Err(Error::internal(
                "bytecode stack crossed a protected super-call pair",
            ));
        }

        if matches!(
            instruction,
            Instruction::Return | Instruction::ReturnDerived(_)
        ) && state
            .regions
            .iter()
            .any(|region| matches!(region, UnwindRegionState::Iterator { .. }))
        {
            return Err(Error::internal("Return bypassed an active iterator close"));
        }
        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;
        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses
        // JS_STACK_SIZE_MAX. This must win over diagnostics from later
        // instructions, including the intentionally truncated call operand in
        // a template with more than 65,535 arguments.
        match instruction {
            // QuickJS terminal completion opcodes consume their completion
            // value and abandon the rest of the frame stack. In particular,
            // `return` and `throw` inside a switch leave its discriminant
            // below that value rather than emitting synthetic cleanup.
            Instruction::Return
            | Instruction::ReturnDerived(_)
            | Instruction::Throw
            | Instruction::Ret => {}
            // QuickJS `OP_throw_error` is terminal and abandons the complete
            // frame stack. A postfix update can legitimately retain its old
            // value below the attempted write when immutable-binding
            // resolution replaces that write with this instruction.
            Instruction::ThrowReadOnly(_)
            | Instruction::ThrowRedeclaration(_)
            | Instruction::ThrowDeleteSuper => {}
            Instruction::Goto(target) => {
                enqueue_target(
                    &mut worklist,
                    *target,
                    VerificationState {
                        depth: next_depth,
                        regions: next_regions,
                        return_addresses: next_return_addresses,
                        super_call_bases: next_super_call_bases,
                    },
                    code.len(),
                )?;
            }
            Instruction::IfFalse(target) | Instruction::IfTrue(target) => {
                let next = VerificationState {
                    depth: next_depth,
                    regions: next_regions,
                    return_addresses: next_return_addresses,
                    super_call_bases: next_super_call_bases,
                };
                enqueue_target(&mut worklist, *target, next.clone(), code.len())?;
                enqueue_fallthrough(&mut worklist, pc, next, code.len())?;
            }
            Instruction::Catch(target) => {
                let exceptional_depth = next_depth;
                record_maximum_depth(&mut maximum, exceptional_depth, declared_max_stack)?;
                enqueue_target(
                    &mut worklist,
                    *target,
                    VerificationState {
                        depth: exceptional_depth,
                        regions: state.regions.clone(),
                        return_addresses: state.return_addresses.clone(),
                        super_call_bases: state.super_call_bases.clone(),
                    },
                    code.len(),
                )?;
                next_regions.push(UnwindRegionState::Catch {
                    target: *target,
                    marker_depth: next_depth,
                });
                enqueue_fallthrough(
                    &mut worklist,
                    pc,
                    VerificationState {
                        depth: next_depth,
                        regions: next_regions,
                        return_addresses: next_return_addresses,
                        super_call_bases: next_super_call_bases,
                    },
                    code.len(),
                )?;
            }
            Instruction::Gosub(target) => {
                let subroutine_depth = state
                    .depth
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("bytecode stack depth overflow"))?;
                record_maximum_depth(&mut maximum, subroutine_depth, declared_max_stack)?;
                enqueue_target(
                    &mut worklist,
                    *target,
                    VerificationState {
                        depth: subroutine_depth,
                        regions: state.regions.clone(),
                        return_addresses: {
                            let mut addresses = state.return_addresses.clone();
                            addresses.push(state.depth);
                            addresses
                        },
                        super_call_bases: state.super_call_bases.clone(),
                    },
                    code.len(),
                )?;
                enqueue_fallthrough(
                    &mut worklist,
                    pc,
                    VerificationState {
                        depth: next_depth,
                        regions: next_regions,
                        return_addresses: next_return_addresses,
                        super_call_bases: next_super_call_bases,
                    },
                    code.len(),
                )?;
            }
            _ => enqueue_fallthrough(
                &mut worklist,
                pc,
                VerificationState {
                    depth: next_depth,
                    regions: next_regions,
                    return_addresses: next_return_addresses,
                    super_call_bases: next_super_call_bases,
                },
                code.len(),
            )?,
        }
    }

    let maximum =
        u16::try_from(maximum).map_err(|_| Error::internal("bytecode stack exceeds u16::MAX"))?;
    Ok(VerifiedBytecode { max_stack: maximum })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum UnwindRegionState {
    Catch {
        target: u32,
        marker_depth: usize,
    },
    Iterator {
        record_base: usize,
        marker_depth: usize,
    },
}

impl UnwindRegionState {
    const fn marker_depth(&self) -> usize {
        match self {
            Self::Catch { marker_depth, .. } | Self::Iterator { marker_depth, .. } => *marker_depth,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VerificationState {
    depth: usize,
    regions: Vec<UnwindRegionState>,
    /// Conceptual operand-stack indexes containing the genuine Int return PCs
    /// introduced by reachable `Gosub` edges. Ordinary opcodes may neither
    /// forge nor consume these typed slots.
    return_addresses: Vec<usize>,
    /// Operand-stack bases of authenticated `super_constructor, new.target`
    /// pairs. Nested `super()` argument expressions form a strict LIFO stack.
    super_call_bases: Vec<usize>,
}

fn verify_super_call_pair_untouched(
    state: &VerificationState,
    remaining_depth: usize,
    popped: usize,
) -> Result<(), Error> {
    if popped > 0
        && state
            .super_call_bases
            .last()
            .is_some_and(|base| remaining_depth < base.saturating_add(2))
    {
        return Err(Error::internal(
            "ordinary bytecode touched a protected super-call pair",
        ));
    }
    Ok(())
}

fn verify_ordinary_consumption(
    state: &VerificationState,
    remaining_depth: usize,
    popped: usize,
) -> Result<(), Error> {
    if popped > 0
        && state
            .return_addresses
            .last()
            .is_some_and(|address| *address >= remaining_depth)
    {
        return Err(Error::internal(
            "ordinary bytecode consumed a gosub return address",
        ));
    }
    if state
        .regions
        .last()
        .is_some_and(|region| remaining_depth < region.marker_depth())
    {
        return Err(Error::internal(
            "bytecode stack crossed an active unwind marker",
        ));
    }
    Ok(())
}

fn record_maximum_depth(
    maximum: &mut usize,
    depth: usize,
    declared_max_stack: u16,
) -> Result<(), Error> {
    *maximum = (*maximum).max(depth);
    if *maximum > usize::from(declared_max_stack) {
        return Err(Error::internal(
            "declared maximum stack is smaller than required",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedBytecode {
    pub max_stack: u16,
}

fn enqueue_target(
    worklist: &mut VecDeque<(usize, VerificationState)>,
    target: u32,
    state: VerificationState,
    code_len: usize,
) -> Result<(), Error> {
    let target = validate_target(target, code_len)?;
    worklist.push_back((target, state));
    Ok(())
}

fn validate_target(target: u32, code_len: usize) -> Result<usize, Error> {
    let target = usize::try_from(target).map_err(|_| Error::internal("jump target overflow"))?;
    if target >= code_len {
        return Err(Error::internal("jump target is out of bounds"));
    }
    Ok(target)
}

fn enqueue_fallthrough(
    worklist: &mut VecDeque<(usize, VerificationState)>,
    pc: usize,
    state: VerificationState,
    code_len: usize,
) -> Result<(), Error> {
    let next = pc
        .checked_add(1)
        .ok_or_else(|| Error::internal("program counter overflow"))?;
    if next >= code_len {
        return Err(Error::internal("bytecode ended without return"));
    }
    worklist.push_back((next, state));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyKind, ArgumentsKind, BytecodeFunction, DefineMethodKind, DynamicEnvironmentSource,
        EvalVariableSource, Instruction, MAX_LOCAL_SLOTS, WithObjectSource,
    };
    use crate::{JsString, Value};

    #[test]
    fn verifier_computes_reachable_stack_depth() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushTrue,
                Instruction::IfFalse(4),
                Instruction::PushI32(1),
                Instruction::Goto(5),
                Instruction::PushI32(2),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(function.verify().unwrap().max_stack, 1);
    }

    #[test]
    fn verifier_models_typed_class_definition_operations() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::CheckCtor,
                Instruction::Undefined,
                Instruction::PushI32(7),
                Instruction::Swap,
                Instruction::Swap,
                Instruction::DefineClass {
                    name: 0,
                    has_heritage: false,
                },
                Instruction::Nip,
                Instruction::Return,
            ],
            constants: vec![Value::String(crate::JsString::from_static("C"))],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(function.verify().unwrap().max_stack, 2);

        let underflow = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::DefineClass {
                    name: 0,
                    has_heritage: false,
                },
                Instruction::Return,
            ],
            max_stack: 2,
            ..function.clone()
        };
        assert_eq!(
            underflow.verify().unwrap_err().message(),
            "bytecode stack underflow"
        );

        let invalid_name = BytecodeFunction {
            constants: vec![Value::Int(0)],
            ..function
        };
        assert_eq!(
            invalid_name.verify().unwrap_err().message(),
            "string-key opcode referenced a non-string constant"
        );

        let heritage = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::PushI32(7),
                Instruction::DefineClass {
                    name: 0,
                    has_heritage: true,
                },
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![Value::String(crate::JsString::from_static("Derived"))],
            local_count: 0,
            max_stack: 2,
            name: None,
        };
        assert_eq!(heritage.verify().unwrap().max_stack, 2);
    }

    #[test]
    fn verifier_models_arguments_creation_as_one_fresh_value() {
        for kind in [ArgumentsKind::Mapped, ArgumentsKind::Unmapped] {
            let function = BytecodeFunction {
                name: None,
                code: vec![Instruction::Arguments(kind), Instruction::Return],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            assert_eq!(function.verify().unwrap().max_stack, 1);
        }

        let undersized = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Arguments(ArgumentsKind::Mapped),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 0,
        };
        assert_eq!(
            undersized.verify().unwrap_err().message(),
            "declared maximum stack is smaller than required"
        );

        let rest = BytecodeFunction {
            name: None,
            code: vec![Instruction::Rest(3), Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(rest.verify().unwrap().max_stack, 1);

        let undersized_rest = BytecodeFunction {
            max_stack: 0,
            ..rest
        };
        assert_eq!(
            undersized_rest.verify().unwrap_err().message(),
            "declared maximum stack is smaller than required"
        );
    }

    #[test]
    fn verifier_models_eval_with_the_quickjs_call_stack_shape() {
        for (argument_count, code, maximum) in [
            (
                0,
                vec![
                    Instruction::Undefined,
                    Instruction::Eval {
                        argument_count: 0,
                        environment: 0,
                    },
                    Instruction::Return,
                ],
                1,
            ),
            (
                2,
                vec![
                    Instruction::Undefined,
                    Instruction::PushI32(1),
                    Instruction::PushI32(2),
                    Instruction::Eval {
                        argument_count: 2,
                        environment: 0,
                    },
                    Instruction::Return,
                ],
                3,
            ),
        ] {
            let function = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 0,
                max_stack: maximum,
            };
            assert_eq!(
                function.verify().unwrap().max_stack,
                maximum,
                "argument count {argument_count}"
            );
        }

        for code in [
            vec![
                Instruction::Eval {
                    argument_count: 0,
                    environment: 0,
                },
                Instruction::Return,
            ],
            vec![
                Instruction::Undefined,
                Instruction::PushI32(1),
                Instruction::Eval {
                    argument_count: 2,
                    environment: 0,
                },
                Instruction::Return,
            ],
        ] {
            let function = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 0,
                max_stack: 2,
            };
            assert_eq!(
                function.verify().unwrap_err().message(),
                "bytecode stack underflow"
            );
        }

        let undersized = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::Eval {
                    argument_count: 2,
                    environment: 0,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(
            undersized.verify().unwrap_err().message(),
            "declared maximum stack is smaller than required"
        );
    }

    #[test]
    fn verifier_models_apply_and_apply_eval_stack_shapes() {
        for (code, maximum) in [
            (
                vec![
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::Apply(ApplyKind::Call),
                    Instruction::Return,
                ],
                3,
            ),
            (
                vec![
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::Apply(ApplyKind::Construct),
                    Instruction::Return,
                ],
                3,
            ),
            (
                vec![
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::ApplyEval { environment: 7 },
                    Instruction::Return,
                ],
                2,
            ),
        ] {
            let function = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 0,
                max_stack: maximum,
            };
            assert_eq!(function.verify().unwrap().max_stack, maximum);
        }

        for instruction in [
            Instruction::Apply(ApplyKind::Call),
            Instruction::Apply(ApplyKind::Construct),
            Instruction::ApplyEval { environment: 0 },
        ] {
            let function = BytecodeFunction {
                name: None,
                code: vec![Instruction::Undefined, instruction, Instruction::Return],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            assert_eq!(
                function.verify().unwrap_err().message(),
                "bytecode stack underflow"
            );
        }
    }

    #[test]
    fn verifier_models_membership_operators_as_binary_booleans() {
        for operator in [Instruction::InstanceOf, Instruction::In] {
            let valid = BytecodeFunction {
                name: None,
                code: vec![
                    Instruction::PushI32(1),
                    Instruction::PushI32(2),
                    operator.clone(),
                    Instruction::Return,
                ],
                constants: vec![],
                local_count: 0,
                max_stack: 2,
            };
            assert_eq!(valid.verify().unwrap().max_stack, 2);

            let underflow = BytecodeFunction {
                name: None,
                code: vec![Instruction::PushI32(1), operator, Instruction::Return],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            assert!(underflow.verify().is_err());
        }
    }

    #[test]
    fn verifier_models_eval_variable_object_stack_and_operands() {
        let source = EvalVariableSource::Local(0);
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::HasEvalVariable { source, name: 0 },
                Instruction::Drop,
                Instruction::GetEvalVariable { source, name: 0 },
                Instruction::Drop,
                Instruction::PushI32(1),
                Instruction::PutEvalVariable { source, name: 0 },
                Instruction::DeleteEvalVariable { source, name: 0 },
                Instruction::Drop,
                Instruction::PushI32(2),
                Instruction::DefineEvalVariable { source, name: 0 },
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![Value::String(JsString::from_static("added"))],
            local_count: 1,
            max_stack: 1,
        };
        assert_eq!(function.verify().unwrap().max_stack, 1);

        for instruction in [
            Instruction::HasEvalVariable { source, name: 0 },
            Instruction::GetEvalVariable { source, name: 0 },
            Instruction::PutEvalVariable { source, name: 0 },
            Instruction::DeleteEvalVariable { source, name: 0 },
            Instruction::DefineEvalVariable { source, name: 0 },
        ] {
            let bad_name = BytecodeFunction {
                name: None,
                code: vec![Instruction::Undefined, Instruction::Return, instruction],
                constants: vec![Value::Int(0)],
                local_count: 1,
                max_stack: 1,
            };
            assert_eq!(
                bad_name.verify().unwrap_err().message(),
                "string-key opcode referenced a non-string constant"
            );
        }

        let bad_local = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::HasEvalVariable { source, name: 0 },
            ],
            constants: vec![Value::String(JsString::from_static("added"))],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(
            bad_local.verify().unwrap_err().message(),
            "eval variable-object local operand is out of bounds"
        );
    }

    #[test]
    fn verifier_models_typed_dynamic_environment_stack_and_operands() {
        let eval = DynamicEnvironmentSource::Eval(EvalVariableSource::Local(0));
        let with = DynamicEnvironmentSource::With(WithObjectSource::Local(1));
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::PushI32(1),
                Instruction::ToObject,
                Instruction::PutLocal(1),
                Instruction::HasDynamicBinding {
                    source: eval,
                    name: 0,
                },
                Instruction::Drop,
                Instruction::GetDynamicBinding {
                    source: with,
                    name: 0,
                },
                Instruction::Drop,
                Instruction::PushI32(2),
                Instruction::PutDynamicBinding {
                    source: eval,
                    name: 0,
                },
                Instruction::DeleteDynamicBinding {
                    source: with,
                    name: 0,
                },
                Instruction::Drop,
                Instruction::DynamicEnvironmentObject(eval),
                Instruction::GetRefValue(0),
                Instruction::PutRefValue(0),
                Instruction::DynamicEnvironmentObject(with),
                Instruction::GetRefValueUndef(0),
                Instruction::PutRefValue(0),
                Instruction::GlobalReference(0),
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![Value::String(JsString::from_static("binding"))],
            local_count: 2,
            max_stack: 2,
        };
        assert_eq!(function.verify().unwrap().max_stack, 2);

        let undersized_global_reference = BytecodeFunction {
            name: None,
            code: vec![Instruction::GlobalReference(0), Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 0,
        };
        assert_eq!(
            undersized_global_reference.verify().unwrap_err().message(),
            "declared maximum stack is smaller than required"
        );

        for instruction in [
            Instruction::HasDynamicBinding {
                source: eval,
                name: 0,
            },
            Instruction::GetDynamicBinding {
                source: eval,
                name: 0,
            },
            Instruction::PutDynamicBinding {
                source: eval,
                name: 0,
            },
            Instruction::DeleteDynamicBinding {
                source: eval,
                name: 0,
            },
            Instruction::GetRefValue(0),
            Instruction::GetRefValueUndef(0),
            Instruction::PutRefValue(0),
        ] {
            let malformed = BytecodeFunction {
                name: None,
                code: vec![Instruction::Undefined, Instruction::Return, instruction],
                constants: vec![Value::Int(0)],
                local_count: 2,
                max_stack: 1,
            };
            assert_eq!(
                malformed.verify().unwrap_err().message(),
                "string-key opcode referenced a non-string constant"
            );
        }

        for source in [
            DynamicEnvironmentSource::Eval(EvalVariableSource::Local(2)),
            DynamicEnvironmentSource::With(WithObjectSource::Local(2)),
        ] {
            let malformed = BytecodeFunction {
                name: None,
                code: vec![
                    Instruction::Undefined,
                    Instruction::Return,
                    Instruction::DynamicEnvironmentObject(source),
                ],
                constants: vec![],
                local_count: 2,
                max_stack: 1,
            };
            assert_eq!(
                malformed.verify().unwrap_err().message(),
                "dynamic environment local operand is out of bounds"
            );
        }
    }

    #[test]
    fn verifier_models_eval_redeclaration_as_a_zero_stack_terminal() {
        let function = BytecodeFunction {
            name: None,
            code: vec![Instruction::ThrowRedeclaration(0)],
            constants: vec![Value::String(JsString::from_static("conflict"))],
            local_count: 0,
            max_stack: 0,
        };
        assert_eq!(function.verify().unwrap().max_stack, 0);

        let malformed = BytecodeFunction {
            name: None,
            code: vec![Instruction::ThrowRedeclaration(0)],
            constants: vec![Value::Int(0)],
            local_count: 0,
            max_stack: 0,
        };
        assert_eq!(
            malformed.verify().unwrap_err().message(),
            "string-key opcode referenced a non-string constant"
        );
    }

    #[test]
    fn verifier_accepts_closed_non_terminating_control_flow() {
        let cycle = BytecodeFunction {
            name: None,
            code: vec![Instruction::Goto(0)],
            constants: vec![],
            local_count: 0,
            max_stack: 0,
        };
        assert_eq!(cycle.verify().unwrap().max_stack, 0);

        let reachable_fallthrough = BytecodeFunction {
            name: None,
            code: vec![Instruction::Nop],
            constants: vec![],
            local_count: 0,
            max_stack: 0,
        };
        assert_eq!(
            reachable_fallthrough.verify().unwrap_err().message(),
            "bytecode ended without return"
        );
    }

    #[test]
    fn verifier_allows_terminal_completion_to_abandon_switch_values() {
        for completion in [Instruction::Return, Instruction::Throw] {
            let function = BytecodeFunction {
                name: None,
                code: vec![Instruction::PushI32(1), Instruction::PushI32(2), completion],
                constants: vec![],
                local_count: 0,
                max_stack: 2,
            };
            assert_eq!(function.verify().unwrap().max_stack, 2);
        }
    }

    #[test]
    fn verifier_tracks_catch_markers_and_exceptional_edges() {
        let normal = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(4),
                Instruction::DropCatch,
                Instruction::PushI32(3),
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(normal.verify().unwrap().max_stack, 1);

        let thrown = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(3),
                Instruction::PushI32(7),
                Instruction::Throw,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(thrown.verify().unwrap().max_stack, 2);

        let nip = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(10),
                Instruction::Catch(6),
                Instruction::PushI32(20),
                Instruction::PushI32(30),
                Instruction::NipCatch,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(nip.verify().unwrap().max_stack, 4);

        for code in [
            vec![
                Instruction::PushI32(1),
                Instruction::DropCatch,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::Catch(4),
                Instruction::PushI32(1),
                Instruction::DropCatch,
                Instruction::Return,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::NipCatch,
                Instruction::Return,
            ],
        ] {
            let malformed = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 0,
                max_stack: 2,
            };
            assert!(malformed.verify().is_err());
        }

        let inconsistent_handlers = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushTrue,
                Instruction::IfFalse(4),
                Instruction::Catch(5),
                Instruction::Goto(5),
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(inconsistent_handlers.verify().is_err());

        let marker_exceeds_declared_stack = BytecodeFunction {
            max_stack: 0,
            ..normal
        };
        assert!(marker_exceeds_declared_stack.verify().is_err());
    }

    #[test]
    fn verifier_tracks_typed_gosub_return_addresses() {
        let returning = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(9),
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::Nop,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(returning.verify().unwrap().max_stack, 2);

        let abrupt_cleanup = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(9),
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::Nop,
                Instruction::DropGosub,
                Instruction::Drop,
                Instruction::PushI32(4),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(abrupt_cleanup.verify().unwrap().max_stack, 2);

        let nip_catch_return_address = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(6),
                Instruction::Undefined,
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::NipCatch,
                Instruction::Ret,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(
            nip_catch_return_address.verify().unwrap_err().message(),
            "NipCatch cannot preserve a gosub return address"
        );

        for code in [
            vec![Instruction::PushI32(0), Instruction::Ret],
            vec![
                Instruction::Catch(3),
                Instruction::Ret,
                Instruction::Nop,
                Instruction::Return,
            ],
            vec![
                Instruction::Gosub(3),
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(0),
                Instruction::DropGosub,
                Instruction::Undefined,
                Instruction::Return,
            ],
        ] {
            let forged = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            assert!(forged.verify().is_err());
        }

        let return_address_exceeds_declared_stack = BytecodeFunction {
            max_stack: 1,
            ..returning
        };
        assert!(return_address_exceeds_declared_stack.verify().is_err());
    }

    #[test]
    fn verifier_dynamic_cleanup_discards_intermediate_gosub_return_addresses() {
        // A return from a finally subroutine can override the pending
        // completion protected by this catch. NipCatch preserves the new
        // return value and intentionally truncates the now-obsolete Gosub PC.
        let nip_catch = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(10),
                Instruction::Undefined,
                Instruction::Gosub(7),
                Instruction::Drop,
                Instruction::DropCatch,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushI32(7),
                Instruction::NipCatch,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(nip_catch.verify().unwrap().max_stack, 4);

        // The same dynamic truncation is required when a return crosses an
        // iterator region while a nested finally Gosub is active.
        let iterator_close = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::Gosub(6),
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushI32(9),
                Instruction::IteratorClosePreserve,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        assert_eq!(iterator_close.verify().unwrap().max_stack, 5);
    }

    #[test]
    fn verifier_tracks_for_of_records_offsets_and_dynamic_close() {
        let normal = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::ForOfNext(0),
                Instruction::Drop,
                Instruction::Drop,
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        assert_eq!(normal.verify().unwrap().max_stack, 5);

        let offset = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(2),
                Instruction::ForOfNext(1),
                Instruction::Drop,
                Instruction::Drop,
                Instruction::Drop,
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 6,
        };
        assert_eq!(offset.verify().unwrap().max_stack, 6);

        let preserve = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(42),
                Instruction::IteratorClosePreserve,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(preserve.verify().unwrap().max_stack, 4);
    }

    #[test]
    fn verifier_tracks_for_in_as_one_ordinary_retained_operand() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForInStart,
                Instruction::ForInNext,
                Instruction::IfFalse(8),
                Instruction::Drop,
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Drop,
                Instruction::Goto(2),
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(function.verify().unwrap().max_stack, 3);

        let underflow = BytecodeFunction {
            name: None,
            code: vec![Instruction::ForInStart, Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(underflow.verify().is_err());
    }

    #[test]
    fn verifier_orders_iterator_catch_and_gosub_regions() {
        // The iterator is inside a catch region. Preserving the return value
        // closes it first, then NipCatch removes the surrounding catch marker.
        let iterator_inside_catch = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(7),
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(9),
                Instruction::IteratorClosePreserve,
                Instruction::NipCatch,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        assert_eq!(iterator_inside_catch.verify().unwrap().max_stack, 5);

        // The catch is inside an iterator. Its exceptional edge retains the
        // outer iterator region, which is then explicitly closed.
        let catch_inside_iterator = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::Catch(7),
                Instruction::DropCatch,
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Drop,
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(catch_inside_iterator.verify().unwrap().max_stack, 4);

        // A finally return PC is below the iterator record. Closing the record
        // must leave that typed address at TOS for Ret.
        let iterator_above_outer_gosub = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Gosub(5),
                Instruction::Return,
                Instruction::Nop,
                Instruction::Nop,
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::IteratorClose,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        assert_eq!(iterator_above_outer_gosub.verify().unwrap().max_stack, 5);

        // Conversely a gosub may temporarily put its address above an outer
        // iterator marker, provided Ret consumes it before iterator cleanup.
        let gosub_above_iterator = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::Gosub(6),
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(gosub_above_iterator.verify().unwrap().max_stack, 4);
    }

    #[test]
    fn verifier_rejects_iterator_marker_and_return_address_misuse() {
        let malformed = [
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::ForOfNext(0),
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::ForOfNext(1),
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::Drop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(2),
                Instruction::IteratorClose,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::IteratorClosePreserve,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(2),
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::DropCatch,
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::Catch(5),
                Instruction::Undefined,
                Instruction::IteratorClose,
                Instruction::Return,
                Instruction::Nop,
                Instruction::Return,
            ],
        ];
        for code in malformed {
            let function = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 0,
                max_stack: 8,
            };
            assert!(function.verify().is_err());
        }

        let preserve_return_address = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::Gosub(6),
                Instruction::IteratorClose,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::IteratorClosePreserve,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(
            preserve_return_address.verify().unwrap_err().message(),
            "IteratorClosePreserve cannot preserve a gosub return address"
        );

        let inconsistent_regions = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushTrue,
                Instruction::IfFalse(5),
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::Goto(8),
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(
            inconsistent_regions.verify().unwrap_err().message(),
            "control flow joins with inconsistent unwind regions"
        );
    }

    #[test]
    fn verifier_rejects_bad_constants_and_stack_joins() {
        let bad_constant = BytecodeFunction {
            name: None,
            code: vec![Instruction::PushConst(0), Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(bad_constant.verify().is_err());

        let runtime_regexp_constant = BytecodeFunction {
            name: None,
            code: vec![Instruction::RegExp(0), Instruction::Return],
            constants: vec![Value::String(JsString::from_static("a"))],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(
            runtime_regexp_constant.verify().unwrap_err().message(),
            "detached bytecode cannot encode RegExp constants"
        );

        let bad_join = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushTrue,
                Instruction::IfFalse(4),
                Instruction::PushI32(1),
                Instruction::Goto(5),
                Instruction::Goto(5),
                Instruction::PushI32(2),
                Instruction::Return,
            ],
            constants: vec![Value::Undefined],
            local_count: 0,
            max_stack: 2,
        };
        assert!(bad_join.verify().is_err());

        let excessive_declared_stack = BytecodeFunction {
            name: None,
            code: vec![Instruction::Undefined, Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: u16::MAX,
        };
        assert!(excessive_declared_stack.verify().is_err());

        let valid_nip = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::Nip,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(valid_nip.verify().unwrap().max_stack, 2);

        let nip_underflow = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::Nip,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(nip_underflow.verify().is_err());

        let postfix_readonly = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PostInc,
                Instruction::ThrowReadOnly(0),
            ],
            constants: vec![Value::String(JsString::from_static("binding"))],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(postfix_readonly.verify().unwrap().max_stack, 2);
    }

    #[test]
    fn verifier_rejects_malformed_operands_even_in_unreachable_code() {
        let bad_constant = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::PushConst(99),
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(bad_constant.verify().is_err());

        let bad_set_name = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::SetName(99),
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(bad_set_name.verify().is_err());

        let non_string_set_name = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::SetName(0),
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            local_count: 0,
            max_stack: 1,
        };
        assert!(non_string_set_name.verify().is_err());

        let non_string_read_only_name = BytecodeFunction {
            name: None,
            code: vec![Instruction::PushI32(1), Instruction::ThrowReadOnly(0)],
            constants: vec![Value::Int(1)],
            local_count: 0,
            max_stack: 1,
        };
        assert!(non_string_read_only_name.verify().is_err());

        let non_string_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::GetField(0),
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            local_count: 0,
            max_stack: 1,
        };
        assert!(non_string_field.verify().is_err());

        let non_string_keep_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::GetField2(0),
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            local_count: 0,
            max_stack: 2,
        };
        assert!(non_string_keep_field.verify().is_err());

        let non_string_put_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::PutField(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![Value::Int(1)],
            local_count: 0,
            max_stack: 2,
        };
        assert!(non_string_put_field.verify().is_err());

        let bad_jump = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Goto(99),
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert!(bad_jump.verify().is_err());

        for instruction in [Instruction::Catch(99), Instruction::Gosub(99)] {
            let bad_handler_target = BytecodeFunction {
                name: None,
                code: vec![Instruction::Undefined, Instruction::Return, instruction],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            assert!(bad_handler_target.verify().is_err());
        }
    }

    #[test]
    fn verifier_enforces_the_declared_quickjs_local_frame() {
        let valid = BytecodeFunction {
            name: None,
            code: vec![Instruction::GetLocal(0), Instruction::Return],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        assert!(valid.verify().is_ok());

        let missing_frame = BytecodeFunction {
            local_count: 0,
            ..valid.clone()
        };
        assert_eq!(
            missing_frame.verify().unwrap_err().message(),
            "local bytecode operand is out of bounds"
        );

        let unreachable = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Return,
                Instruction::GetLocal(0),
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(
            unreachable.verify().unwrap_err().message(),
            "local bytecode operand is out of bounds"
        );

        let last_slot = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::GetLocal(MAX_LOCAL_SLOTS - 1),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: MAX_LOCAL_SLOTS,
            max_stack: 1,
        };
        assert!(last_slot.verify().is_ok());

        let excessive_frame = BytecodeFunction {
            name: None,
            code: vec![Instruction::Undefined, Instruction::Return],
            constants: vec![],
            local_count: u16::MAX,
            max_stack: 1,
        };
        assert_eq!(
            excessive_frame.verify().unwrap_err().message(),
            "declared local count exceeds QuickJS JS_MAX_LOCAL_VARS"
        );
    }

    #[test]
    fn verifier_models_quickjs_lexical_frame_operations() {
        let initialized_then_updated = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::PushI32(1),
                Instruction::InitializeLocal(0),
                Instruction::GetLocalCheck(0),
                Instruction::PushI32(1),
                Instruction::Add,
                Instruction::SetLocalCheck(0),
                Instruction::CloseLocal(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 2,
        };
        assert_eq!(initialized_then_updated.verify().unwrap().max_stack, 2);

        let consuming_write = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::PushI32(1),
                Instruction::InitializeLocal(0),
                Instruction::PushI32(2),
                Instruction::PutLocalCheck(0),
                Instruction::GetLocalCheck(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        assert_eq!(consuming_write.verify().unwrap().max_stack, 1);

        let checked_var_ref = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::GetVarRefCheck(0),
                Instruction::PutVarRefCheck(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(checked_var_ref.verify().unwrap().max_stack, 1);

        for instruction in [
            Instruction::InitializeLocal(0),
            Instruction::InitializeDerivedLocal(0),
            Instruction::PutLocalCheck(0),
            Instruction::SetLocalCheck(0),
            Instruction::PutVarRefCheck(0),
            Instruction::InitializeDerivedVarRef(0),
        ] {
            let underflow = BytecodeFunction {
                name: None,
                code: vec![instruction, Instruction::Undefined, Instruction::Return],
                constants: vec![],
                local_count: 1,
                max_stack: 1,
            };
            assert!(underflow.verify().is_err());
        }
    }

    #[test]
    fn verifier_rejects_every_lexical_local_operand_outside_the_frame() {
        for instruction in [
            Instruction::SetLocalUninitialized(0),
            Instruction::GetLocalCheck(0),
            Instruction::InitializeLocal(0),
            Instruction::InitializeDerivedLocal(0),
            Instruction::PutLocalCheck(0),
            Instruction::SetLocalCheck(0),
            Instruction::CloseLocal(0),
            Instruction::ReturnDerived(0),
        ] {
            let function = BytecodeFunction {
                name: None,
                code: vec![Instruction::Undefined, Instruction::Return, instruction],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            assert_eq!(
                function.verify().unwrap_err().message(),
                "local bytecode operand is out of bounds"
            );
        }
    }

    #[test]
    fn verifier_models_typed_derived_constructor_operations() {
        let default_derived = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushActiveFunction,
                Instruction::PutLocal(1),
                Instruction::InitDerivedConstructor,
                Instruction::Dup,
                Instruction::InitializeDerivedLocal(0),
                Instruction::ReturnDerived(0),
            ],
            constants: vec![],
            local_count: 2,
            max_stack: 2,
        };
        assert_eq!(default_derived.verify().unwrap().max_stack, 2);

        let captured_initializer = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::InitializeDerivedVarRef(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(captured_initializer.verify().unwrap().max_stack, 1);

        let return_underflow = BytecodeFunction {
            code: vec![Instruction::ReturnDerived(0)],
            local_count: 1,
            max_stack: 0,
            ..BytecodeFunction::default()
        };
        assert_eq!(
            return_underflow.verify().unwrap_err().message(),
            "bytecode stack underflow"
        );

        // ReturnDerived is a terminal completion and must not make its
        // unreachable successor part of the control-flow graph.
        let terminal = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::ReturnDerived(0),
                Instruction::PushI32(1),
            ],
            local_count: 1,
            max_stack: 1,
            ..BytecodeFunction::default()
        };
        assert_eq!(terminal.verify().unwrap().max_stack, 1);
    }

    #[test]
    fn verifier_protects_typed_super_call_pairs_across_control_flow() {
        let conditional_argument = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::MarkSuperCall,
                Instruction::PushTrue,
                Instruction::IfFalse(7),
                Instruction::PushI32(1),
                Instruction::Goto(8),
                Instruction::PushI32(2),
                Instruction::ConstructSuper(1),
                Instruction::Return,
            ],
            max_stack: 3,
            ..BytecodeFunction::default()
        };
        assert_eq!(conditional_argument.verify().unwrap().max_stack, 3);

        let nested = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::MarkSuperCall,
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::MarkSuperCall,
                Instruction::ConstructSuper(0),
                Instruction::ConstructSuper(1),
                Instruction::Return,
            ],
            max_stack: 4,
            ..BytecodeFunction::default()
        };
        assert_eq!(nested.verify().unwrap().max_stack, 4);

        let spread = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::MarkSuperCall,
                Instruction::ArrayFrom(0),
                Instruction::ApplySuper,
                Instruction::Return,
            ],
            max_stack: 3,
            ..BytecodeFunction::default()
        };
        assert_eq!(spread.verify().unwrap().max_stack, 3);

        let unmarked = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::ConstructSuper(0),
                Instruction::Return,
            ],
            max_stack: 2,
            ..BytecodeFunction::default()
        };
        assert_eq!(
            unmarked.verify().unwrap_err().message(),
            "typed super construction did not consume its protected pair"
        );

        let wrong_arity = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::MarkSuperCall,
                Instruction::Undefined,
                Instruction::ConstructSuper(0),
                Instruction::Return,
            ],
            max_stack: 3,
            ..BytecodeFunction::default()
        };
        assert_eq!(
            wrong_arity.verify().unwrap_err().message(),
            "typed super construction did not consume its protected pair"
        );

        let generic_apply = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::MarkSuperCall,
                Instruction::ArrayFrom(0),
                Instruction::Apply(ApplyKind::Construct),
                Instruction::Return,
            ],
            max_stack: 3,
            ..BytecodeFunction::default()
        };
        assert_eq!(
            generic_apply.verify().unwrap_err().message(),
            "ordinary bytecode touched a protected super-call pair"
        );

        for touching in [
            Instruction::Swap,
            Instruction::Drop,
            Instruction::Construct(0),
        ] {
            let malformed = BytecodeFunction {
                code: vec![
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::MarkSuperCall,
                    touching,
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                max_stack: 2,
                ..BytecodeFunction::default()
            };
            assert_eq!(
                malformed.verify().unwrap_err().message(),
                "ordinary bytecode touched a protected super-call pair"
            );
        }

        let inconsistent_join = BytecodeFunction {
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::PushTrue,
                Instruction::IfFalse(6),
                Instruction::MarkSuperCall,
                Instruction::Goto(7),
                Instruction::Nop,
                Instruction::Nop,
                Instruction::Undefined,
                Instruction::Return,
            ],
            max_stack: 3,
            ..BytecodeFunction::default()
        };
        assert_eq!(
            inconsistent_join.verify().unwrap_err().message(),
            "control flow joins with inconsistent super-call markers"
        );
    }

    #[test]
    fn verifier_models_array_literal_stack_contracts() {
        let fixed = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::ArrayFrom(2),
                Instruction::PushI32(3),
                Instruction::DefineField(0),
                Instruction::Return,
            ],
            constants: vec![Value::String(crate::value::JsString::from_static("2"))],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(fixed.verify().unwrap().max_stack, 2);

        let dynamic = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::ArrayFrom(0),
                Instruction::PushI32(0),
                Instruction::Undefined,
                Instruction::DefineArrayEl,
                Instruction::Inc,
                Instruction::Undefined,
                Instruction::Append,
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(dynamic.verify().unwrap().max_stack, 3);

        let trailing_hole = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::ArrayFrom(0),
                Instruction::PushI32(2),
                Instruction::Dup1,
                Instruction::PutField(0),
                Instruction::Return,
            ],
            constants: vec![Value::String(crate::value::JsString::from_static("length"))],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(trailing_hole.verify().unwrap().max_stack, 3);
    }

    #[test]
    fn verifier_models_object_literal_stack_contracts() {
        let fixed_proto_spread = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Object,
                Instruction::PushI32(1),
                Instruction::DefineField(0),
                Instruction::Undefined,
                Instruction::DefineMethod {
                    key: 1,
                    kind: DefineMethodKind::Method,
                    enumerable: true,
                },
                Instruction::Null,
                Instruction::SetProto,
                Instruction::Undefined,
                Instruction::CopyDataProperties,
                Instruction::Return,
            ],
            constants: vec![
                Value::String(crate::value::JsString::from_static("field")),
                Value::String(crate::value::JsString::from_static("method")),
            ],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(fixed_proto_spread.verify().unwrap().max_stack, 2);

        let computed = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Object,
                Instruction::PushI32(1),
                Instruction::ToPropKey,
                Instruction::Undefined,
                Instruction::DefineMethodComputed {
                    kind: DefineMethodKind::Getter,
                    enumerable: true,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(computed.verify().unwrap().max_stack, 3);
    }

    #[test]
    fn verifier_models_depth_addressed_object_rest_copy_without_stack_mutation() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1), // excluded
                Instruction::PushI32(2), // source
                Instruction::PushI32(3), // prepared Reference
                Instruction::PushI32(4), // target
                Instruction::CopyDataPropertiesExcluded {
                    target_depth: 0,
                    source_depth: 2,
                    excluded_depth: 3,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(function.verify().unwrap().max_stack, 4);

        let underflow = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::PushI32(3),
                Instruction::CopyDataPropertiesExcluded {
                    target_depth: 0,
                    source_depth: 2,
                    excluded_depth: 3,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(
            underflow.verify().unwrap_err().message(),
            "bytecode stack underflow"
        );
    }

    #[test]
    fn verifier_rejects_object_literal_opcode_underflow() {
        for instruction in [
            Instruction::SetNameComputed,
            Instruction::DefineMethod {
                key: 0,
                kind: DefineMethodKind::Method,
                enumerable: true,
            },
            Instruction::DefineMethodComputed {
                kind: DefineMethodKind::Setter,
                enumerable: true,
            },
            Instruction::SetProto,
            Instruction::CopyDataProperties,
        ] {
            let constants = if matches!(&instruction, Instruction::DefineMethod { .. }) {
                vec![Value::String(crate::value::JsString::from_static("method"))]
            } else {
                vec![]
            };
            let function = BytecodeFunction {
                name: None,
                code: vec![instruction, Instruction::Return],
                constants,
                local_count: 0,
                max_stack: 2,
            };
            assert_eq!(
                function.verify().unwrap_err().message(),
                "bytecode stack underflow"
            );
        }
    }

    #[test]
    fn verifier_rejects_malformed_array_literal_operands() {
        let non_string_field = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineField(0),
                Instruction::Return,
            ],
            constants: vec![Value::Int(0)],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(
            non_string_field.verify().unwrap_err().message(),
            "string-key opcode referenced a non-string constant"
        );

        let non_string_method = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineMethod {
                    key: 0,
                    kind: DefineMethodKind::Method,
                    enumerable: true,
                },
                Instruction::Return,
            ],
            constants: vec![Value::Int(0)],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(
            non_string_method.verify().unwrap_err().message(),
            "string-key opcode referenced a non-string constant"
        );

        let array_from_underflow = BytecodeFunction {
            name: None,
            code: vec![Instruction::ArrayFrom(1), Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(
            array_from_underflow.verify().unwrap_err().message(),
            "bytecode stack underflow"
        );
    }
}

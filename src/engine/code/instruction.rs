//! Shared instruction facts. Runtime-specific region and resume checks remain
//! in their verifiers; a nominal pop/push pair cannot prove those invariants.
use super::bytecode::{
    ApplyKind, ArgumentsKind, DefineMethodKind, DynamicEnvironmentSource, EvalVariableSource,
    Instruction, IteratorCallKind, PrivateNameSource,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StackEffect {
    pub popped: usize,
    pub pushed: usize,
    pub state: StackStateEffect,
}

/// Symbolic transitions which require the verifier's current typed state.
/// Nominal counts remain available for underflow checking, but these protocols
/// may restore a recorded depth, preserve a completion, or add resume payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackStateEffect {
    Ordinary,
    MarkSuperCall,
    ConsumeSuperCall,
    EnterCatch,
    DropCatch,
    PreserveCatch,
    EnterIterator,
    AdvanceIterator,
    FinishIteratorValue,
    CloseIterator,
    PreserveIterator,
    DetachIterator,
    EnterGosub,
    ConsumeGosub,
    ResumeYield,
    ResumeAwait,
}

impl StackStateEffect {
    pub(crate) const fn preserves_super_pair(self) -> bool {
        !matches!(
            self,
            Self::ConsumeSuperCall | Self::AdvanceIterator | Self::FinishIteratorValue
        )
    }
}

/// Catchable JavaScript completion, excluding allocation/invariant engine errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsExceptionEffect {
    None,
    MayThrow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlEffect {
    Next,
    Jump(u32),
    Branch(u32),
    Catch(u32),
    Gosub(u32),
    Ret,
    Return,
    TailCall,
    Throw,
    Suspend,
}

impl ControlEffect {
    pub(crate) const fn target(self) -> Option<u32> {
        match self {
            Self::Jump(target)
            | Self::Branch(target)
            | Self::Catch(target)
            | Self::Gosub(target) => Some(target),
            Self::Next
            | Self::Ret
            | Self::Return
            | Self::TailCall
            | Self::Throw
            | Self::Suspend => None,
        }
    }
    pub(crate) const fn ends_block(self) -> bool {
        !matches!(self, Self::Next)
    }
}

/// Constant-pool roles do not replace type/permission validation of the payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstantRole {
    Value,
    Function,
    RegExp,
    StaticName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StackDepthRole {
    Target,
    Source,
    Excluded,
}

/// Encoded operands retain semantic units; a slot is never confused with a PC,
/// constant-pool index, count, or authenticated hidden source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operand {
    Integer(i32),
    AtomValueIndex(u32),
    Constant { index: u32, role: ConstantRole },
    Local(u16),
    Argument(u16),
    Closure(u16),
    PrivateLocal(u16),
    ArgumentCount(u16),
    RestStart(u16),
    ElementCount(u16),
    Target(u32),
    EvalEnvironment(u16),
    EvalSource(EvalVariableSource),
    DynamicSource(DynamicEnvironmentSource),
    PrivateSource(PrivateNameSource),
    ArgumentsKind(ArgumentsKind),
    ApplyKind(ApplyKind),
    IteratorCallKind(IteratorCallKind),
    MethodKind(DefineMethodKind),
    Enumerable(bool),
    HasHeritage(bool),
    IteratorDepth(u8),
    StackDepth { role: StackDepthRole, depth: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OperandContract(pub [Option<Operand>; 3]);

impl OperandContract {
    pub(crate) const fn static_name(self) -> Option<u32> {
        let mut index = 0;
        while index < self.0.len() {
            if let Some(Operand::Constant {
                index: constant,
                role: ConstantRole::StaticName,
            }) = self.0[index]
            {
                return Some(constant);
            }
            index += 1;
        }
        None
    }

    pub(crate) const fn eval_environment(self) -> Option<u16> {
        let mut index = 0;
        while index < self.0.len() {
            if let Some(Operand::EvalEnvironment(environment)) = self.0[index] {
                return Some(environment);
            }
            index += 1;
        }
        None
    }
}

/// Conservative generic-language effects. Allocation means managed values or
/// their storage, not VM window growth or reference-accounting bookkeeping.
/// A false bit is a guarantee; a true bit permits an effect and is not a claim
/// that every operand tag takes that path. Number fast paths refine these facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PotentialEffects {
    pub javascript_exception: JsExceptionEffect,
    pub may_call_js: bool,
    pub may_allocate: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InstructionInfo {
    pub stack: StackEffect,
    pub effects: PotentialEffects,
    pub control: ControlEffect,
    pub operands: OperandContract,
}

impl Instruction {
    pub(crate) const fn info(&self) -> InstructionInfo {
        InstructionInfo {
            stack: self.stack_contract(),
            effects: self.potential_effects(),
            control: self.control_effect(),
            operands: self.operand_contract(),
        }
    }

    /// Read only the needed part of the canonical contract. Verification and
    /// block discovery must not construct unrelated operand/effect payloads.
    pub(crate) const fn stack_contract(&self) -> StackEffect {
        let (popped, pushed) = self.nominal_stack_effect();
        StackEffect {
            popped,
            pushed,
            state: self.stack_state_effect(),
        }
    }

    pub(crate) const fn potential_effects(&self) -> PotentialEffects {
        PotentialEffects {
            javascript_exception: self.javascript_exception_effect(),
            may_call_js: self.may_call_js(),
            may_allocate: self.may_allocate(),
        }
    }

    #[must_use]
    pub(crate) const fn nominal_stack_effect(&self) -> (usize, usize) {
        match self {
            Self::Nop
            | Self::CheckCtor
            | Self::Goto(_)
            | Self::Gosub(_)
            | Self::ReturnUndefined
            | Self::ThrowRedeclaration(_)
            | Self::ThrowReadOnly(_)
            | Self::ThrowIteratorMissingThrow
            | Self::SetLocalUninitialized(_)
            | Self::CloseLocal(_)
            | Self::InitialYield => (0, 0),
            // The verifier models this marker in its conceptual stack even
            // though runtime execution stores it in private handler metadata.
            Self::Catch(_) => (0, 1),
            Self::ForOfStart | Self::ForAwaitOfStart => (1, 3),
            Self::IteratorStart | Self::AsyncIteratorStart => (1, 3),
            Self::ForOfNext(_) => (3, 5),
            Self::ForAwaitOfNext => (3, 4),
            Self::IteratorGetValueDone => (2, 3),
            Self::IteratorNext => (4, 4),
            Self::IteratorCall(_) => (4, 5),
            Self::ForInStart => (1, 1),
            Self::ForInNext => (1, 3),
            Self::PushI32(_)
            | Self::PushAtomValueIndex(_)
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
            Self::InitializePrivateName(_) => (0, 0),
            Self::InitializePrivateMethod(_) | Self::InitializePrivateAccessor(_) => (2, 1),
            Self::SetName(_) | Self::ToObject | Self::IteratorCheckObject => (1, 1),
            Self::MarkSuperCall => (2, 2),
            Self::GetRefValue(_) | Self::GetRefValueUndef(_) => (1, 2),
            Self::GetField(_) | Self::GetPrivateField(_) | Self::PrivateIn(_) => (1, 1),
            Self::GetField2(_) => (1, 2),
            Self::GetPrivateField2(_) => (1, 2),
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
            Self::PutPrivateField(_) => (2, 0),
            Self::PutArrayEl => (3, 0),
            Self::PutSuperValue => (4, 0),
            Self::DefineField(_) | Self::DefinePrivateField(_) | Self::DefineMethod { .. } => {
                (2, 1)
            }
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
            Self::Import => (2, 1),
            Self::Call(argument_count) | Self::Eval { argument_count, .. } => {
                (*argument_count as usize + 1, 1)
            }
            Self::TailCall(argument_count) => (*argument_count as usize + 1, 0),
            Self::CallMethod(argument_count) => (*argument_count as usize + 2, 1),
            Self::TailCallMethod(argument_count) => (*argument_count as usize + 2, 0),
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
            | Self::InitializeVarRef(_)
            | Self::InitializeModuleImportCollision(_)
            | Self::InitializeDerivedVarRef(_)
            | Self::PutVar(_)
            | Self::PutVarInit(_)
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
            Self::NipCatch | Self::IteratorClosePreserve | Self::IteratorDropPreserve => (1, 1),
            // The verifier replaces this nominal completion-to-completion-plus-
            // iterator effect with the active region's dynamic record base.
            Self::IteratorDetachPreserve => (1, 2),
            Self::IteratorClose => (3, 0),
            Self::Await => (1, 1),
            Self::Yield | Self::YieldStar | Self::AsyncYieldStar => (1, 2),
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
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction => (1, 1),
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

    const fn javascript_exception_effect(&self) -> JsExceptionEffect {
        match self {
            Self::Nop
            | Self::PushI32(..)
            | Self::PushAtomValueIndex(..)
            | Self::PushConst(..)
            | Self::FClosure(..)
            | Self::RegExp(..)
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::Arguments(..)
            | Self::Rest(..)
            | Self::VariableEnvironment
            | Self::GetLocal(..)
            | Self::PutLocal(..)
            | Self::SetLocal(..)
            | Self::SetLocalUninitialized(..)
            | Self::InitializeLocal(..)
            | Self::GetArg(..)
            | Self::PutArg(..)
            | Self::SetArg(..)
            | Self::GetVarRef(..)
            | Self::PutVarRef(..)
            | Self::SetVarRef(..)
            | Self::InitializeVarRef(..)
            | Self::InitializeModuleImportCollision(..)
            | Self::CloseLocal(..)
            | Self::InitializePrivateName(..)
            | Self::ArrayFrom(..)
            | Self::Object
            | Self::Insert2
            | Self::Insert3
            | Self::Dup3
            | Self::Insert4
            | Self::Perm3
            | Self::Perm4
            | Self::Perm5
            | Self::Rot4Left
            | Self::Drop
            | Self::Nip
            | Self::Swap
            | Self::Dup
            | Self::Dup1
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction
            | Self::StrictEq
            | Self::StrictNeq
            | Self::InitialYield
            | Self::MarkSuperCall => JsExceptionEffect::None,
            Self::SetName(..)
            | Self::ThrowReadOnly(..)
            | Self::ThrowRedeclaration(..)
            | Self::ThrowDeleteSuper
            | Self::HasEvalVariable { .. }
            | Self::GetEvalVariable { .. }
            | Self::PutEvalVariable { .. }
            | Self::DeleteEvalVariable { .. }
            | Self::DefineEvalVariable { .. }
            | Self::ToObject
            | Self::HasDynamicBinding { .. }
            | Self::GetDynamicBinding { .. }
            | Self::PutDynamicBinding { .. }
            | Self::DeleteDynamicBinding { .. }
            | Self::DynamicEnvironmentObject(..)
            | Self::GlobalReference(..)
            | Self::GetRefValue(..)
            | Self::GetRefValueUndef(..)
            | Self::PutRefValue(..)
            | Self::GetLocalCheck(..)
            | Self::InitializeDerivedLocal(..)
            | Self::PutLocalCheck(..)
            | Self::SetLocalCheck(..)
            | Self::GetVarRefCheck(..)
            | Self::PutVarRefCheck(..)
            | Self::InitializeDerivedVarRef(..)
            | Self::GetVar(..)
            | Self::GetVarUndef(..)
            | Self::DeleteVar(..)
            | Self::PutVar(..)
            | Self::PutVarInit(..)
            | Self::InitializePrivateMethod(..)
            | Self::InitializePrivateAccessor(..)
            | Self::GetPrivateField(..)
            | Self::GetPrivateField2(..)
            | Self::PutPrivateField(..)
            | Self::DefinePrivateField(..)
            | Self::PrivateIn(..)
            | Self::GetField(..)
            | Self::GetField2(..)
            | Self::GetArrayEl
            | Self::GetArrayEl2
            | Self::GetArrayEl3
            | Self::GetSuper
            | Self::GetSuperValue
            | Self::GetSuperValueForCall
            | Self::ToPropKey
            | Self::PutField(..)
            | Self::PutArrayEl
            | Self::PutSuperValue
            | Self::DefineField(..)
            | Self::DefineFieldComputed
            | Self::DefineMethod { .. }
            | Self::DefineMethodComputed { .. }
            | Self::DefineClass { .. }
            | Self::InstallClassInstanceInitializer
            | Self::CallClassInstanceInitializer
            | Self::RunClassStaticInitializer
            | Self::CallClassStaticBlock
            | Self::DefineArrayEl
            | Self::SetNameComputed
            | Self::SetProto
            | Self::CopyDataProperties
            | Self::CopyDataPropertiesExcluded { .. }
            | Self::Append
            | Self::Delete
            | Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::PostInc
            | Self::PostDec
            | Self::BitNot
            | Self::Add
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
            | Self::Neq
            | Self::Lt
            | Self::Lte
            | Self::Gt
            | Self::Gte
            | Self::InstanceOf
            | Self::In
            | Self::IfFalse(..)
            | Self::IfTrue(..)
            | Self::Goto(..)
            | Self::Catch(..)
            | Self::DropCatch
            | Self::NipCatch
            | Self::Gosub(..)
            | Self::Ret
            | Self::DropGosub
            | Self::ForOfStart
            | Self::ForAwaitOfStart
            | Self::ForOfNext(..)
            | Self::ForAwaitOfNext
            | Self::IteratorGetValueDone
            | Self::ForInStart
            | Self::ForInNext
            | Self::IteratorClose
            | Self::IteratorClosePreserve
            | Self::IteratorDropPreserve
            | Self::IteratorDetachPreserve
            | Self::IteratorStart
            | Self::AsyncIteratorStart
            | Self::IteratorNext
            | Self::IteratorCall(..)
            | Self::IteratorCheckObject
            | Self::Yield
            | Self::YieldStar
            | Self::AsyncYieldStar
            | Self::Await
            | Self::ThrowIteratorMissingThrow
            | Self::Import
            | Self::Call(..)
            | Self::TailCall(..)
            | Self::Eval { .. }
            | Self::CallMethod(..)
            | Self::TailCallMethod(..)
            | Self::Construct(..)
            | Self::ConstructSuper(..)
            | Self::CheckCtor
            | Self::InitDerivedConstructor
            | Self::Apply(..)
            | Self::ApplySuper
            | Self::ApplyEval { .. }
            | Self::Return
            | Self::ReturnUndefined
            | Self::ReturnDerived(..)
            | Self::Throw => JsExceptionEffect::MayThrow,
        }
    }

    pub(crate) const fn control_effect(&self) -> ControlEffect {
        match self {
            Self::IfFalse(target) => ControlEffect::Branch(*target),
            Self::IfTrue(target) => ControlEffect::Branch(*target),
            Self::Goto(target) => ControlEffect::Jump(*target),
            Self::Catch(target) => ControlEffect::Catch(*target),
            Self::Gosub(target) => ControlEffect::Gosub(*target),
            Self::Nop
            | Self::PushI32(..)
            | Self::PushAtomValueIndex(..)
            | Self::PushConst(..)
            | Self::FClosure(..)
            | Self::RegExp(..)
            | Self::SetName(..)
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::Arguments(..)
            | Self::Rest(..)
            | Self::VariableEnvironment
            | Self::HasEvalVariable { .. }
            | Self::GetEvalVariable { .. }
            | Self::PutEvalVariable { .. }
            | Self::DeleteEvalVariable { .. }
            | Self::DefineEvalVariable { .. }
            | Self::ToObject
            | Self::HasDynamicBinding { .. }
            | Self::GetDynamicBinding { .. }
            | Self::PutDynamicBinding { .. }
            | Self::DeleteDynamicBinding { .. }
            | Self::DynamicEnvironmentObject(..)
            | Self::GlobalReference(..)
            | Self::GetRefValue(..)
            | Self::GetRefValueUndef(..)
            | Self::PutRefValue(..)
            | Self::GetLocal(..)
            | Self::PutLocal(..)
            | Self::SetLocal(..)
            | Self::SetLocalUninitialized(..)
            | Self::GetLocalCheck(..)
            | Self::InitializeLocal(..)
            | Self::InitializeDerivedLocal(..)
            | Self::PutLocalCheck(..)
            | Self::SetLocalCheck(..)
            | Self::GetArg(..)
            | Self::PutArg(..)
            | Self::SetArg(..)
            | Self::GetVarRef(..)
            | Self::PutVarRef(..)
            | Self::SetVarRef(..)
            | Self::GetVarRefCheck(..)
            | Self::PutVarRefCheck(..)
            | Self::InitializeVarRef(..)
            | Self::InitializeModuleImportCollision(..)
            | Self::InitializeDerivedVarRef(..)
            | Self::CloseLocal(..)
            | Self::GetVar(..)
            | Self::GetVarUndef(..)
            | Self::DeleteVar(..)
            | Self::PutVar(..)
            | Self::PutVarInit(..)
            | Self::InitializePrivateName(..)
            | Self::InitializePrivateMethod(..)
            | Self::InitializePrivateAccessor(..)
            | Self::GetPrivateField(..)
            | Self::GetPrivateField2(..)
            | Self::PutPrivateField(..)
            | Self::DefinePrivateField(..)
            | Self::PrivateIn(..)
            | Self::GetField(..)
            | Self::GetField2(..)
            | Self::GetArrayEl
            | Self::GetArrayEl2
            | Self::GetArrayEl3
            | Self::GetSuper
            | Self::GetSuperValue
            | Self::GetSuperValueForCall
            | Self::ArrayFrom(..)
            | Self::Object
            | Self::ToPropKey
            | Self::Insert2
            | Self::Insert3
            | Self::Dup3
            | Self::Insert4
            | Self::Perm3
            | Self::Perm4
            | Self::Perm5
            | Self::Rot4Left
            | Self::PutField(..)
            | Self::PutArrayEl
            | Self::PutSuperValue
            | Self::DefineField(..)
            | Self::DefineFieldComputed
            | Self::DefineMethod { .. }
            | Self::DefineMethodComputed { .. }
            | Self::DefineClass { .. }
            | Self::InstallClassInstanceInitializer
            | Self::CallClassInstanceInitializer
            | Self::RunClassStaticInitializer
            | Self::CallClassStaticBlock
            | Self::DefineArrayEl
            | Self::SetNameComputed
            | Self::SetProto
            | Self::CopyDataProperties
            | Self::CopyDataPropertiesExcluded { .. }
            | Self::Append
            | Self::Delete
            | Self::Drop
            | Self::Nip
            | Self::Swap
            | Self::Dup
            | Self::Dup1
            | Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::PostInc
            | Self::PostDec
            | Self::BitNot
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction
            | Self::Add
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
            | Self::InstanceOf
            | Self::In
            | Self::DropCatch
            | Self::NipCatch
            | Self::DropGosub
            | Self::ForOfStart
            | Self::ForAwaitOfStart
            | Self::ForOfNext(..)
            | Self::ForAwaitOfNext
            | Self::IteratorGetValueDone
            | Self::ForInStart
            | Self::ForInNext
            | Self::IteratorClose
            | Self::IteratorClosePreserve
            | Self::IteratorDropPreserve
            | Self::IteratorDetachPreserve
            | Self::IteratorStart
            | Self::AsyncIteratorStart
            | Self::IteratorNext
            | Self::IteratorCall(..)
            | Self::IteratorCheckObject
            | Self::Import
            | Self::Call(..)
            | Self::Eval { .. }
            | Self::CallMethod(..)
            | Self::Construct(..)
            | Self::MarkSuperCall
            | Self::ConstructSuper(..)
            | Self::CheckCtor
            | Self::InitDerivedConstructor
            | Self::Apply(..)
            | Self::ApplySuper
            | Self::ApplyEval { .. } => ControlEffect::Next,
            Self::Ret => ControlEffect::Ret,
            Self::Return | Self::ReturnUndefined | Self::ReturnDerived(..) => ControlEffect::Return,
            Self::TailCall(..) | Self::TailCallMethod(..) => ControlEffect::TailCall,
            Self::ThrowReadOnly(..)
            | Self::ThrowRedeclaration(..)
            | Self::ThrowDeleteSuper
            | Self::ThrowIteratorMissingThrow
            | Self::Throw => ControlEffect::Throw,
            Self::InitialYield
            | Self::Yield
            | Self::YieldStar
            | Self::AsyncYieldStar
            | Self::Await => ControlEffect::Suspend,
        }
    }
    pub(crate) const fn operand_contract(&self) -> OperandContract {
        use Operand as O;
        match self {
            Self::PushI32(value) => OperandContract([Some(O::Integer(*value)), None, None]),
            Self::PushAtomValueIndex(value) => {
                OperandContract([Some(O::AtomValueIndex(*value)), None, None])
            }
            Self::PushConst(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::Value,
                }),
                None,
                None,
            ]),
            Self::FClosure(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::Function,
                }),
                None,
                None,
            ]),
            Self::RegExp(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::RegExp,
                }),
                None,
                None,
            ]),
            Self::SetName(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::ThrowReadOnly(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::ThrowRedeclaration(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::Arguments(value) => OperandContract([Some(O::ArgumentsKind(*value)), None, None]),
            Self::Rest(value) => OperandContract([Some(O::RestStart(*value)), None, None]),
            Self::HasEvalVariable { source, name } => OperandContract([
                Some(O::EvalSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::GetEvalVariable { source, name } => OperandContract([
                Some(O::EvalSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::PutEvalVariable { source, name } => OperandContract([
                Some(O::EvalSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::DeleteEvalVariable { source, name } => OperandContract([
                Some(O::EvalSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::DefineEvalVariable { source, name } => OperandContract([
                Some(O::EvalSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::HasDynamicBinding { source, name } => OperandContract([
                Some(O::DynamicSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::GetDynamicBinding { source, name } => OperandContract([
                Some(O::DynamicSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::PutDynamicBinding { source, name } => OperandContract([
                Some(O::DynamicSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::DeleteDynamicBinding { source, name } => OperandContract([
                Some(O::DynamicSource(*source)),
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                None,
            ]),
            Self::DynamicEnvironmentObject(value) => {
                OperandContract([Some(O::DynamicSource(*value)), None, None])
            }
            Self::GlobalReference(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::GetRefValue(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::GetRefValueUndef(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::PutRefValue(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::GetLocal(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::PutLocal(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::SetLocal(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::SetLocalUninitialized(value) => {
                OperandContract([Some(O::Local(*value)), None, None])
            }
            Self::GetLocalCheck(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::InitializeLocal(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::InitializeDerivedLocal(value) => {
                OperandContract([Some(O::Local(*value)), None, None])
            }
            Self::PutLocalCheck(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::SetLocalCheck(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::GetArg(value) => OperandContract([Some(O::Argument(*value)), None, None]),
            Self::PutArg(value) => OperandContract([Some(O::Argument(*value)), None, None]),
            Self::SetArg(value) => OperandContract([Some(O::Argument(*value)), None, None]),
            Self::GetVarRef(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::PutVarRef(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::SetVarRef(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::GetVarRefCheck(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::PutVarRefCheck(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::InitializeVarRef(value) => {
                OperandContract([Some(O::Closure(*value)), None, None])
            }
            Self::InitializeModuleImportCollision(value) => {
                OperandContract([Some(O::Closure(*value)), None, None])
            }
            Self::InitializeDerivedVarRef(value) => {
                OperandContract([Some(O::Closure(*value)), None, None])
            }
            Self::CloseLocal(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::GetVar(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::GetVarUndef(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::DeleteVar(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::PutVar(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::PutVarInit(value) => OperandContract([Some(O::Closure(*value)), None, None]),
            Self::InitializePrivateName(value) => {
                OperandContract([Some(O::PrivateLocal(*value)), None, None])
            }
            Self::InitializePrivateMethod(value) => {
                OperandContract([Some(O::PrivateLocal(*value)), None, None])
            }
            Self::InitializePrivateAccessor(value) => {
                OperandContract([Some(O::PrivateLocal(*value)), None, None])
            }
            Self::GetPrivateField(value) => {
                OperandContract([Some(O::PrivateSource(*value)), None, None])
            }
            Self::GetPrivateField2(value) => {
                OperandContract([Some(O::PrivateSource(*value)), None, None])
            }
            Self::PutPrivateField(value) => {
                OperandContract([Some(O::PrivateSource(*value)), None, None])
            }
            Self::DefinePrivateField(value) => {
                OperandContract([Some(O::PrivateSource(*value)), None, None])
            }
            Self::PrivateIn(value) => OperandContract([Some(O::PrivateSource(*value)), None, None]),
            Self::GetField(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::GetField2(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::ArrayFrom(value) => OperandContract([Some(O::ElementCount(*value)), None, None]),
            Self::PutField(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::DefineField(value) => OperandContract([
                Some(O::Constant {
                    index: *value,
                    role: ConstantRole::StaticName,
                }),
                None,
                None,
            ]),
            Self::DefineMethod {
                key,
                kind,
                enumerable,
            } => OperandContract([
                Some(O::Constant {
                    index: *key,
                    role: ConstantRole::StaticName,
                }),
                Some(O::MethodKind(*kind)),
                Some(O::Enumerable(*enumerable)),
            ]),
            Self::DefineMethodComputed { kind, enumerable } => OperandContract([
                Some(O::MethodKind(*kind)),
                Some(O::Enumerable(*enumerable)),
                None,
            ]),
            Self::DefineClass { name, has_heritage } => OperandContract([
                Some(O::Constant {
                    index: *name,
                    role: ConstantRole::StaticName,
                }),
                Some(O::HasHeritage(*has_heritage)),
                None,
            ]),
            Self::CopyDataPropertiesExcluded {
                target_depth,
                source_depth,
                excluded_depth,
            } => OperandContract([
                Some(O::StackDepth {
                    role: StackDepthRole::Target,
                    depth: *target_depth,
                }),
                Some(O::StackDepth {
                    role: StackDepthRole::Source,
                    depth: *source_depth,
                }),
                Some(O::StackDepth {
                    role: StackDepthRole::Excluded,
                    depth: *excluded_depth,
                }),
            ]),
            Self::IfFalse(value) => OperandContract([Some(O::Target(*value)), None, None]),
            Self::IfTrue(value) => OperandContract([Some(O::Target(*value)), None, None]),
            Self::Goto(value) => OperandContract([Some(O::Target(*value)), None, None]),
            Self::Catch(value) => OperandContract([Some(O::Target(*value)), None, None]),
            Self::Gosub(value) => OperandContract([Some(O::Target(*value)), None, None]),
            Self::ForOfNext(value) => OperandContract([Some(O::IteratorDepth(*value)), None, None]),
            Self::IteratorCall(value) => {
                OperandContract([Some(O::IteratorCallKind(*value)), None, None])
            }
            Self::Call(value) => OperandContract([Some(O::ArgumentCount(*value)), None, None]),
            Self::TailCall(value) => OperandContract([Some(O::ArgumentCount(*value)), None, None]),
            Self::Eval {
                argument_count,
                environment,
            } => OperandContract([
                Some(O::ArgumentCount(*argument_count)),
                Some(O::EvalEnvironment(*environment)),
                None,
            ]),
            Self::CallMethod(value) => {
                OperandContract([Some(O::ArgumentCount(*value)), None, None])
            }
            Self::TailCallMethod(value) => {
                OperandContract([Some(O::ArgumentCount(*value)), None, None])
            }
            Self::Construct(value) => OperandContract([Some(O::ArgumentCount(*value)), None, None]),
            Self::ConstructSuper(value) => {
                OperandContract([Some(O::ArgumentCount(*value)), None, None])
            }
            Self::Apply(value) => OperandContract([Some(O::ApplyKind(*value)), None, None]),
            Self::ApplyEval { environment } => {
                OperandContract([Some(O::EvalEnvironment(*environment)), None, None])
            }
            Self::ReturnDerived(value) => OperandContract([Some(O::Local(*value)), None, None]),
            Self::Nop
            | Self::ThrowDeleteSuper
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::VariableEnvironment
            | Self::ToObject
            | Self::GetArrayEl
            | Self::GetArrayEl2
            | Self::GetArrayEl3
            | Self::GetSuper
            | Self::GetSuperValue
            | Self::GetSuperValueForCall
            | Self::Object
            | Self::ToPropKey
            | Self::Insert2
            | Self::Insert3
            | Self::Dup3
            | Self::Insert4
            | Self::Perm3
            | Self::Perm4
            | Self::Perm5
            | Self::Rot4Left
            | Self::PutArrayEl
            | Self::PutSuperValue
            | Self::DefineFieldComputed
            | Self::InstallClassInstanceInitializer
            | Self::CallClassInstanceInitializer
            | Self::RunClassStaticInitializer
            | Self::CallClassStaticBlock
            | Self::DefineArrayEl
            | Self::SetNameComputed
            | Self::SetProto
            | Self::CopyDataProperties
            | Self::Append
            | Self::Delete
            | Self::Drop
            | Self::Nip
            | Self::Swap
            | Self::Dup
            | Self::Dup1
            | Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::PostInc
            | Self::PostDec
            | Self::BitNot
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction
            | Self::Add
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
            | Self::InstanceOf
            | Self::In
            | Self::DropCatch
            | Self::NipCatch
            | Self::Ret
            | Self::DropGosub
            | Self::ForOfStart
            | Self::ForAwaitOfStart
            | Self::ForAwaitOfNext
            | Self::IteratorGetValueDone
            | Self::ForInStart
            | Self::ForInNext
            | Self::IteratorClose
            | Self::IteratorClosePreserve
            | Self::IteratorDropPreserve
            | Self::IteratorDetachPreserve
            | Self::IteratorStart
            | Self::AsyncIteratorStart
            | Self::IteratorNext
            | Self::IteratorCheckObject
            | Self::InitialYield
            | Self::Yield
            | Self::YieldStar
            | Self::AsyncYieldStar
            | Self::Await
            | Self::ThrowIteratorMissingThrow
            | Self::Import
            | Self::MarkSuperCall
            | Self::CheckCtor
            | Self::InitDerivedConstructor
            | Self::ApplySuper
            | Self::Return
            | Self::ReturnUndefined
            | Self::Throw => OperandContract([None; 3]),
        }
    }
    const fn may_call_js(&self) -> bool {
        match self {
            Self::Nop
            | Self::PushI32(..)
            | Self::PushAtomValueIndex(..)
            | Self::PushConst(..)
            | Self::FClosure(..)
            | Self::RegExp(..)
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::Arguments(..)
            | Self::Rest(..)
            | Self::VariableEnvironment
            | Self::DynamicEnvironmentObject(..)
            | Self::GetLocal(..)
            | Self::PutLocal(..)
            | Self::SetLocal(..)
            | Self::SetLocalUninitialized(..)
            | Self::GetLocalCheck(..)
            | Self::InitializeLocal(..)
            | Self::InitializeDerivedLocal(..)
            | Self::PutLocalCheck(..)
            | Self::SetLocalCheck(..)
            | Self::GetArg(..)
            | Self::PutArg(..)
            | Self::SetArg(..)
            | Self::GetVarRef(..)
            | Self::PutVarRef(..)
            | Self::SetVarRef(..)
            | Self::GetVarRefCheck(..)
            | Self::PutVarRefCheck(..)
            | Self::InitializeVarRef(..)
            | Self::InitializeModuleImportCollision(..)
            | Self::InitializeDerivedVarRef(..)
            | Self::CloseLocal(..)
            | Self::InitializePrivateName(..)
            | Self::ArrayFrom(..)
            | Self::Object
            | Self::Insert2
            | Self::Insert3
            | Self::Dup3
            | Self::Insert4
            | Self::Perm3
            | Self::Perm4
            | Self::Perm5
            | Self::Rot4Left
            | Self::Drop
            | Self::Nip
            | Self::Swap
            | Self::Dup
            | Self::Dup1
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction
            | Self::StrictEq
            | Self::StrictNeq
            | Self::IfFalse(..)
            | Self::IfTrue(..)
            | Self::Goto(..)
            | Self::Catch(..)
            | Self::DropCatch
            | Self::NipCatch
            | Self::Gosub(..)
            | Self::Ret
            | Self::DropGosub
            | Self::InitialYield
            | Self::MarkSuperCall
            | Self::Return
            | Self::ReturnUndefined => false,
            Self::SetName(..)
            | Self::ThrowReadOnly(..)
            | Self::ThrowRedeclaration(..)
            | Self::ThrowDeleteSuper
            | Self::HasEvalVariable { .. }
            | Self::GetEvalVariable { .. }
            | Self::PutEvalVariable { .. }
            | Self::DeleteEvalVariable { .. }
            | Self::DefineEvalVariable { .. }
            | Self::ToObject
            | Self::HasDynamicBinding { .. }
            | Self::GetDynamicBinding { .. }
            | Self::PutDynamicBinding { .. }
            | Self::DeleteDynamicBinding { .. }
            | Self::GlobalReference(..)
            | Self::GetRefValue(..)
            | Self::GetRefValueUndef(..)
            | Self::PutRefValue(..)
            | Self::GetVar(..)
            | Self::GetVarUndef(..)
            | Self::DeleteVar(..)
            | Self::PutVar(..)
            | Self::PutVarInit(..)
            | Self::InitializePrivateMethod(..)
            | Self::InitializePrivateAccessor(..)
            | Self::GetPrivateField(..)
            | Self::GetPrivateField2(..)
            | Self::PutPrivateField(..)
            | Self::DefinePrivateField(..)
            | Self::PrivateIn(..)
            | Self::GetField(..)
            | Self::GetField2(..)
            | Self::GetArrayEl
            | Self::GetArrayEl2
            | Self::GetArrayEl3
            | Self::GetSuper
            | Self::GetSuperValue
            | Self::GetSuperValueForCall
            | Self::ToPropKey
            | Self::PutField(..)
            | Self::PutArrayEl
            | Self::PutSuperValue
            | Self::DefineField(..)
            | Self::DefineFieldComputed
            | Self::DefineMethod { .. }
            | Self::DefineMethodComputed { .. }
            | Self::DefineClass { .. }
            | Self::InstallClassInstanceInitializer
            | Self::CallClassInstanceInitializer
            | Self::RunClassStaticInitializer
            | Self::CallClassStaticBlock
            | Self::DefineArrayEl
            | Self::SetNameComputed
            | Self::SetProto
            | Self::CopyDataProperties
            | Self::CopyDataPropertiesExcluded { .. }
            | Self::Append
            | Self::Delete
            | Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::PostInc
            | Self::PostDec
            | Self::BitNot
            | Self::Add
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
            | Self::Neq
            | Self::Lt
            | Self::Lte
            | Self::Gt
            | Self::Gte
            | Self::InstanceOf
            | Self::In
            | Self::ForOfStart
            | Self::ForAwaitOfStart
            | Self::ForOfNext(..)
            | Self::ForAwaitOfNext
            | Self::IteratorGetValueDone
            | Self::ForInStart
            | Self::ForInNext
            | Self::IteratorClose
            | Self::IteratorClosePreserve
            | Self::IteratorDropPreserve
            | Self::IteratorDetachPreserve
            | Self::IteratorStart
            | Self::AsyncIteratorStart
            | Self::IteratorNext
            | Self::IteratorCall(..)
            | Self::IteratorCheckObject
            | Self::Yield
            | Self::YieldStar
            | Self::AsyncYieldStar
            | Self::Await
            | Self::ThrowIteratorMissingThrow
            | Self::Import
            | Self::Call(..)
            | Self::TailCall(..)
            | Self::Eval { .. }
            | Self::CallMethod(..)
            | Self::TailCallMethod(..)
            | Self::Construct(..)
            | Self::ConstructSuper(..)
            | Self::CheckCtor
            | Self::InitDerivedConstructor
            | Self::Apply(..)
            | Self::ApplySuper
            | Self::ApplyEval { .. }
            | Self::ReturnDerived(..)
            | Self::Throw => true,
        }
    }
    const fn may_allocate(&self) -> bool {
        match self {
            Self::Nop
            | Self::PushI32(..)
            | Self::PushConst(..)
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::GetLocal(..)
            | Self::PutLocal(..)
            | Self::SetLocal(..)
            | Self::SetLocalUninitialized(..)
            | Self::GetArg(..)
            | Self::PutArg(..)
            | Self::SetArg(..)
            | Self::GetVarRef(..)
            | Self::PutVarRef(..)
            | Self::SetVarRef(..)
            | Self::Insert2
            | Self::Insert3
            | Self::Dup3
            | Self::Insert4
            | Self::Perm3
            | Self::Perm4
            | Self::Perm5
            | Self::Rot4Left
            | Self::Drop
            | Self::Nip
            | Self::Swap
            | Self::Dup
            | Self::Dup1
            | Self::Not
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction
            | Self::StrictEq
            | Self::StrictNeq
            | Self::IfFalse(..)
            | Self::IfTrue(..)
            | Self::Goto(..)
            | Self::Catch(..)
            | Self::DropCatch
            | Self::NipCatch
            | Self::Gosub(..)
            | Self::Ret
            | Self::DropGosub
            | Self::InitialYield
            | Self::MarkSuperCall
            | Self::Return
            | Self::ReturnUndefined => false,
            Self::PushAtomValueIndex(..)
            | Self::FClosure(..)
            | Self::RegExp(..)
            | Self::SetName(..)
            | Self::ThrowReadOnly(..)
            | Self::ThrowRedeclaration(..)
            | Self::ThrowDeleteSuper
            | Self::PushThis
            | Self::Arguments(..)
            | Self::Rest(..)
            | Self::VariableEnvironment
            | Self::HasEvalVariable { .. }
            | Self::GetEvalVariable { .. }
            | Self::PutEvalVariable { .. }
            | Self::DeleteEvalVariable { .. }
            | Self::DefineEvalVariable { .. }
            | Self::ToObject
            | Self::HasDynamicBinding { .. }
            | Self::GetDynamicBinding { .. }
            | Self::PutDynamicBinding { .. }
            | Self::DeleteDynamicBinding { .. }
            | Self::DynamicEnvironmentObject(..)
            | Self::GlobalReference(..)
            | Self::GetRefValue(..)
            | Self::GetRefValueUndef(..)
            | Self::PutRefValue(..)
            | Self::GetLocalCheck(..)
            | Self::InitializeLocal(..)
            | Self::InitializeDerivedLocal(..)
            | Self::PutLocalCheck(..)
            | Self::SetLocalCheck(..)
            | Self::GetVarRefCheck(..)
            | Self::PutVarRefCheck(..)
            | Self::InitializeVarRef(..)
            | Self::InitializeModuleImportCollision(..)
            | Self::InitializeDerivedVarRef(..)
            | Self::CloseLocal(..)
            | Self::GetVar(..)
            | Self::GetVarUndef(..)
            | Self::DeleteVar(..)
            | Self::PutVar(..)
            | Self::PutVarInit(..)
            | Self::InitializePrivateName(..)
            | Self::InitializePrivateMethod(..)
            | Self::InitializePrivateAccessor(..)
            | Self::GetPrivateField(..)
            | Self::GetPrivateField2(..)
            | Self::PutPrivateField(..)
            | Self::DefinePrivateField(..)
            | Self::PrivateIn(..)
            | Self::GetField(..)
            | Self::GetField2(..)
            | Self::GetArrayEl
            | Self::GetArrayEl2
            | Self::GetArrayEl3
            | Self::GetSuper
            | Self::GetSuperValue
            | Self::GetSuperValueForCall
            | Self::ArrayFrom(..)
            | Self::Object
            | Self::ToPropKey
            | Self::PutField(..)
            | Self::PutArrayEl
            | Self::PutSuperValue
            | Self::DefineField(..)
            | Self::DefineFieldComputed
            | Self::DefineMethod { .. }
            | Self::DefineMethodComputed { .. }
            | Self::DefineClass { .. }
            | Self::InstallClassInstanceInitializer
            | Self::CallClassInstanceInitializer
            | Self::RunClassStaticInitializer
            | Self::CallClassStaticBlock
            | Self::DefineArrayEl
            | Self::SetNameComputed
            | Self::SetProto
            | Self::CopyDataProperties
            | Self::CopyDataPropertiesExcluded { .. }
            | Self::Append
            | Self::Delete
            | Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::PostInc
            | Self::PostDec
            | Self::BitNot
            | Self::TypeOf
            | Self::Add
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
            | Self::Neq
            | Self::Lt
            | Self::Lte
            | Self::Gt
            | Self::Gte
            | Self::InstanceOf
            | Self::In
            | Self::ForOfStart
            | Self::ForAwaitOfStart
            | Self::ForOfNext(..)
            | Self::ForAwaitOfNext
            | Self::IteratorGetValueDone
            | Self::ForInStart
            | Self::ForInNext
            | Self::IteratorClose
            | Self::IteratorClosePreserve
            | Self::IteratorDropPreserve
            | Self::IteratorDetachPreserve
            | Self::IteratorStart
            | Self::AsyncIteratorStart
            | Self::IteratorNext
            | Self::IteratorCall(..)
            | Self::IteratorCheckObject
            | Self::Yield
            | Self::YieldStar
            | Self::AsyncYieldStar
            | Self::Await
            | Self::ThrowIteratorMissingThrow
            | Self::Import
            | Self::Call(..)
            | Self::TailCall(..)
            | Self::Eval { .. }
            | Self::CallMethod(..)
            | Self::TailCallMethod(..)
            | Self::Construct(..)
            | Self::ConstructSuper(..)
            | Self::CheckCtor
            | Self::InitDerivedConstructor
            | Self::Apply(..)
            | Self::ApplySuper
            | Self::ApplyEval { .. }
            | Self::ReturnDerived(..)
            | Self::Throw => true,
        }
    }
    const fn stack_state_effect(&self) -> StackStateEffect {
        match self {
            Self::Nop
            | Self::PushI32(..)
            | Self::PushAtomValueIndex(..)
            | Self::PushConst(..)
            | Self::FClosure(..)
            | Self::RegExp(..)
            | Self::SetName(..)
            | Self::ThrowReadOnly(..)
            | Self::ThrowRedeclaration(..)
            | Self::ThrowDeleteSuper
            | Self::Undefined
            | Self::Null
            | Self::PushFalse
            | Self::PushTrue
            | Self::PushThis
            | Self::PushActiveFunction
            | Self::PushHomeObject
            | Self::PushNewTarget
            | Self::Arguments(..)
            | Self::Rest(..)
            | Self::VariableEnvironment
            | Self::HasEvalVariable { .. }
            | Self::GetEvalVariable { .. }
            | Self::PutEvalVariable { .. }
            | Self::DeleteEvalVariable { .. }
            | Self::DefineEvalVariable { .. }
            | Self::ToObject
            | Self::HasDynamicBinding { .. }
            | Self::GetDynamicBinding { .. }
            | Self::PutDynamicBinding { .. }
            | Self::DeleteDynamicBinding { .. }
            | Self::DynamicEnvironmentObject(..)
            | Self::GlobalReference(..)
            | Self::GetRefValue(..)
            | Self::GetRefValueUndef(..)
            | Self::PutRefValue(..)
            | Self::GetLocal(..)
            | Self::PutLocal(..)
            | Self::SetLocal(..)
            | Self::SetLocalUninitialized(..)
            | Self::GetLocalCheck(..)
            | Self::InitializeLocal(..)
            | Self::InitializeDerivedLocal(..)
            | Self::PutLocalCheck(..)
            | Self::SetLocalCheck(..)
            | Self::GetArg(..)
            | Self::PutArg(..)
            | Self::SetArg(..)
            | Self::GetVarRef(..)
            | Self::PutVarRef(..)
            | Self::SetVarRef(..)
            | Self::GetVarRefCheck(..)
            | Self::PutVarRefCheck(..)
            | Self::InitializeVarRef(..)
            | Self::InitializeModuleImportCollision(..)
            | Self::InitializeDerivedVarRef(..)
            | Self::CloseLocal(..)
            | Self::GetVar(..)
            | Self::GetVarUndef(..)
            | Self::DeleteVar(..)
            | Self::PutVar(..)
            | Self::PutVarInit(..)
            | Self::InitializePrivateName(..)
            | Self::InitializePrivateMethod(..)
            | Self::InitializePrivateAccessor(..)
            | Self::GetPrivateField(..)
            | Self::GetPrivateField2(..)
            | Self::PutPrivateField(..)
            | Self::DefinePrivateField(..)
            | Self::PrivateIn(..)
            | Self::GetField(..)
            | Self::GetField2(..)
            | Self::GetArrayEl
            | Self::GetArrayEl2
            | Self::GetArrayEl3
            | Self::GetSuper
            | Self::GetSuperValue
            | Self::GetSuperValueForCall
            | Self::ArrayFrom(..)
            | Self::Object
            | Self::ToPropKey
            | Self::Insert2
            | Self::Insert3
            | Self::Dup3
            | Self::Insert4
            | Self::Perm3
            | Self::Perm4
            | Self::Perm5
            | Self::Rot4Left
            | Self::PutField(..)
            | Self::PutArrayEl
            | Self::PutSuperValue
            | Self::DefineField(..)
            | Self::DefineFieldComputed
            | Self::DefineMethod { .. }
            | Self::DefineMethodComputed { .. }
            | Self::DefineClass { .. }
            | Self::InstallClassInstanceInitializer
            | Self::CallClassInstanceInitializer
            | Self::RunClassStaticInitializer
            | Self::CallClassStaticBlock
            | Self::DefineArrayEl
            | Self::SetNameComputed
            | Self::SetProto
            | Self::CopyDataProperties
            | Self::CopyDataPropertiesExcluded { .. }
            | Self::Append
            | Self::Delete
            | Self::Drop
            | Self::Nip
            | Self::Swap
            | Self::Dup
            | Self::Dup1
            | Self::Neg
            | Self::Plus
            | Self::Inc
            | Self::Dec
            | Self::PostInc
            | Self::PostDec
            | Self::BitNot
            | Self::Not
            | Self::TypeOf
            | Self::IsUndefinedOrNull
            | Self::IsUndefined
            | Self::IsNull
            | Self::TypeOfIsUndefined
            | Self::TypeOfIsFunction
            | Self::Add
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
            | Self::InstanceOf
            | Self::In
            | Self::IfFalse(..)
            | Self::IfTrue(..)
            | Self::Goto(..)
            | Self::ForInStart
            | Self::ForInNext
            | Self::IteratorStart
            | Self::AsyncIteratorStart
            | Self::IteratorNext
            | Self::IteratorCall(..)
            | Self::IteratorCheckObject
            | Self::ThrowIteratorMissingThrow
            | Self::Import
            | Self::Call(..)
            | Self::TailCall(..)
            | Self::Eval { .. }
            | Self::CallMethod(..)
            | Self::TailCallMethod(..)
            | Self::Construct(..)
            | Self::CheckCtor
            | Self::InitDerivedConstructor
            | Self::Apply(..)
            | Self::ApplyEval { .. }
            | Self::Return
            | Self::ReturnUndefined
            | Self::ReturnDerived(..)
            | Self::Throw => StackStateEffect::Ordinary,
            Self::MarkSuperCall => StackStateEffect::MarkSuperCall,
            Self::ConstructSuper(..) | Self::ApplySuper => StackStateEffect::ConsumeSuperCall,
            Self::Catch(..) => StackStateEffect::EnterCatch,
            Self::DropCatch => StackStateEffect::DropCatch,
            Self::NipCatch => StackStateEffect::PreserveCatch,
            Self::ForOfStart | Self::ForAwaitOfStart => StackStateEffect::EnterIterator,
            Self::ForOfNext(..) | Self::ForAwaitOfNext => StackStateEffect::AdvanceIterator,
            Self::IteratorGetValueDone => StackStateEffect::FinishIteratorValue,
            Self::IteratorClose => StackStateEffect::CloseIterator,
            Self::IteratorClosePreserve | Self::IteratorDropPreserve => {
                StackStateEffect::PreserveIterator
            }
            Self::IteratorDetachPreserve => StackStateEffect::DetachIterator,
            Self::Gosub(..) => StackStateEffect::EnterGosub,
            Self::Ret | Self::DropGosub => StackStateEffect::ConsumeGosub,
            Self::InitialYield | Self::Yield | Self::YieldStar | Self::AsyncYieldStar => {
                StackStateEffect::ResumeYield
            }
            Self::Await => StackStateEffect::ResumeAwait,
        }
    }
}

/// Render the actual final instruction stream with the same facts consumed by
/// verification and optimization. Formatting is diagnostic-only.
#[cfg(feature = "profiling")]
pub(crate) fn disassemble(code: &[Instruction]) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for (pc, instruction) in code.iter().enumerate() {
        writeln!(
            &mut output,
            "{pc:05} {instruction:?} ; {:?}",
            instruction.info()
        )
        .expect("writing to a String cannot fail");
    }
    output
}

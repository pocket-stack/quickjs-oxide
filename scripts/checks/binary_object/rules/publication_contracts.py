"""Authenticated publication ownership and refactoring-equivalent instruction facts.

The instruction fact hash is deliberately the pre-split reviewed hash. Only the
explicit projection refactor below is reversed, after authenticating its bridges.
Instrumentation is removable only when it matches the exact inert timer shape.
"""
import re


def normalized(ctx, source):
    return " ".join(ctx.rust_code_only(source).split())


def require(ctx, rule, description, actual, expected):
    if re.sub(r"\s+", "", ctx.rust_code_only(actual)) != re.sub(r"\s+", "", ctx.rust_code_only(expected)):
        ctx.fail(rule, description)


def function(ctx, source, name, rule):
    return ctx.unique_braced_item(
        ctx.rust_code_only(source),
        re.compile(r"(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?fn\s+" + name + r"\b[^{}]*\{"),
        rule, name,
    )[0]


def check_verified(ctx):
    rule = "published-function-verification"
    source = ctx.read_source("src/engine/code/verify/verified.rs")
    timer = '''#[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Verify,
        );'''
    code = normalized(ctx, source)
    marker = normalized(ctx, timer)
    if code.count(marker) not in (0, 4):
        ctx.fail(rule, "profiling must be absent or use the exact timer in all four authenticated publication roles")
    # Removing precisely this non-control-flow statement leaves the original
    # private owning wrapper, all role checks, and consumption of that same draft.
    ctx.require_normalized_code_sha256(
        rule, "role-specific verification must precede ownership of the exact draft",
        code.replace(marker, ""),
        "1c0f9ca9afbe05a4525057b1f0d456fd8d42f858e995fd714bd202edf15a281e",
    )


def check_instruction(ctx):
    rule = "published-instruction-contract"
    source = ctx.read_source("src/engine/code/instruction.rs")
    bridges = {
        "info": '''pub(crate) const fn info(&self) -> InstructionInfo {
            InstructionInfo { stack: self.stack_contract(), effects: self.potential_effects(),
                control: self.control_effect(), operands: self.operand_contract(), }
        }''',
        "stack_contract": '''pub(crate) const fn stack_contract(&self) -> StackEffect {
            let (popped, pushed) = self.nominal_stack_effect();
            StackEffect { popped, pushed, state: self.stack_state_effect(), }
        }''',
        "potential_effects": '''pub(crate) const fn potential_effects(&self) -> PotentialEffects {
            PotentialEffects { javascript_exception: self.javascript_exception_effect(),
                may_call_js: self.may_call_js(), may_allocate: self.may_allocate(), }
        }''',
    }
    split = bool(re.search(r"\bfn\s+stack_contract\b", ctx.rust_code_only(source)))
    found = {}
    for name, expected in (bridges.items() if split else ()):
        found[name] = function(ctx, source, name, rule)
        require(ctx, rule, f"{name} must project the same canonical facts without branches or substitutions", found[name], expected)
    old_info = '''pub(crate) const fn info(&self) -> InstructionInfo {
        let (popped, pushed) = self.nominal_stack_effect();
        InstructionInfo {
            stack: StackEffect { popped, pushed, state: self.stack_state_effect(), },
            effects: PotentialEffects { javascript_exception: self.javascript_exception_effect(),
                may_call_js: self.may_call_js(), may_allocate: self.may_allocate(), },
            control: self.control_effect(), operands: self.operand_contract(),
        }
    }'''
    code = normalized(ctx, source)
    for name in ("stack_contract", "potential_effects"):
        if found.get(name):
            code = code.replace(normalized(ctx, found[name]), "", 1)
    if found.get("info"):
        code = code.replace(normalized(ctx, found["info"]), normalized(ctx, old_info), 1)
    for name in (("nominal_stack_effect", "control_effect", "operand_contract") if split else ()):
        code = code.replace(f"pub(crate) const fn {name}", f"const fn {name}", 1)
    ctx.require_normalized_code_sha256(
        rule, "splitting projections must not change any reviewed opcode/operand/control/effect facts",
        code, "b3f1a9fe0e3fdd46ec6a0a89f24b6403fd79802663ea0e4e8032d5992bcf4289",
    )
    rule = "published-instruction-stack-adapter"
    require(ctx, rule, "the public tuple must directly use the canonical nominal stack producer",
        function(ctx, ctx.read_source("src/engine/code/bytecode.rs"), "stack_effect", rule),
        '''pub const fn stack_effect(&self) -> (usize, usize) { self.nominal_stack_effect() }''' if split else '''pub const fn stack_effect(&self) -> (usize, usize) { let effect = self.info().stack; (effect.popped, effect.pushed) }''')


def check_executable(ctx):
    rule = "published-executable-owner"
    source = ctx.read_source("src/engine/code/executable.rs")
    if "root: std::cell::OnceCell<FunctionBytecodeRef>" in source:
        # S10 reviewed ownership model: the heap caches only immutable Rc data
        # and a publication certificate; the owning callee keeps the bytecode
        # alive. Root acquisition validates Runtime at the observation boundary.
        # Authenticate BOTH the sealed projection and its sole witness producer:
        # from_authentication is sound only with select's domain/generation/
        # closure checks. No mutable IC/value exception is inferred by matching
        # field names; the S12 position-table constructor is part of this hash.
        production = source.split("#[cfg(test)]\nmod tests", 1)[0]
        ctx.require_normalized_code_sha256(
            rule, "lazy publication must retain sealed fields, checked root construction, immutable data and fixture-only mutation",
            ctx.rust_code_only(production), "d6ba64b8a8cec7d1c766334ac9b59c8bd6dd2375b3b8e9c2466f0ff246846c47",
        )
        witness = ctx.read_source("src/engine/vm/call/ordinary.rs").split("#[cfg(test)]\nmod tests", 1)[0]
        ctx.require_normalized_code_sha256(
            rule, "rootless certificate construction requires same-runtime, exact publication and closure checks before witness installation",
            ctx.rust_code_only(witness), "8aae056f3d2572ad39525e430e53cdabbcdd16a1e4ec36db807fa0cf3050ed18",
        )
        return
    # Item checks exclude the standalone tests module, but explicitly include
    # the two cfg(test) mutation/fixture escape hatches and authenticate their guards.
    production = source.split("#[cfg(test)]\nmod tests", 1)[0]
    code = ctx.rust_code_only(production)
    owned = "fn snapshot_function_bytecode_owned" in production
    cached = "data:Rc<PublishedFunctionData>," in re.sub(r"\s+", "", code)
    for name, expected in {
        "PublishedFunctionSnapshot": "root: Option<FunctionBytecodeRef>, data: " + ("Rc<PublishedFunctionData>" if cached else "PublishedFunctionData") + ",",
        "PublishedEvalEnvironment": "owner: FunctionBytecodeRef, environments: Rc<[EvalEnvironment<Atom>]>, index: usize,",
    }.items():
        item = ctx.unique_braced_item(code, re.compile(r"pub\(crate\)\s+struct\s+" + name + r"\s*\{"), rule, name)[0]
        require(ctx, rule, f"{name} must expose no public owner or mutable data field", item,
            "pub(crate) struct " + name + " { " + expected + " }")
    for owner, target, expression in (
        ("PublishedFunctionSnapshot", "PublishedFunctionData", "&self.data"),
        ("PublishedEvalEnvironment", "EvalEnvironment<Atom>", "&self.environments[self.index]"),
    ):
        item = ctx.unique_braced_item(code, re.compile(r"impl\s+std::ops::Deref\s+for\s+" + owner + r"\s*\{"), rule, owner + " immutable Deref")[0]
        require(ctx, rule, "Deref must borrow its owner's immutable selected storage", item,
            "impl std::ops::Deref for " + owner + " { type Target = " + target + "; fn deref(&self) -> &Self::Target { " + expression + " } }")
    data = ctx.unique_braced_item(code, re.compile(r"pub\(crate\)\s+struct\s+PublishedFunctionData\s*\{"), rule, "cached data fields")[0]
    require(ctx, rule, "cached metadata must own immutable Rc arrays, not a rooting cycle or mutable shared cell", data, PUBLISHED_DATA.replace("pub(crate) struct PublishedFunctionData {", "pub(crate) struct PublishedFunctionData { pub(crate) observes_arguments: bool,") if owned else PUBLISHED_DATA)
    if re.findall(r"\bfn\s+(\w+)\s*(?:<[^{}]*>)?\s*\(", code) != [
        "same_environment", "owner", "deref", "deref", "frame_layout", "constant",
        "eval_environment", "root", "empty_for_test", "deref_mut", "snapshot_function_bytecode",
    ] + (["snapshot_function_bytecode_owned"] if owned else []):
        ctx.fail(rule, "publication projection must not introduce an alternate constructor or mutator")
    for name, expected in EXECUTABLE_FUNCTIONS.items():
        require(ctx, rule, f"{name} must retain checked same-owner immutable projection",
            function(ctx, production, name, rule), expected)
    if not re.search(r"#\[cfg\(test\)\]\s+pub\(crate\)\s+fn\s+empty_for_test", code):
        ctx.fail(rule, "unrooted fixture construction must remain test-only")
    mutable = ctx.unique_braced_item(code, re.compile(r"impl\s+std::ops::DerefMut\s+for\s+PublishedFunctionSnapshot\s*\{"), rule, "fixture mutable projection")[0]
    require(ctx, rule, "mutable fixture projection must reject real owners and require unique Rc storage", mutable,
        '''impl std::ops::DerefMut for PublishedFunctionSnapshot {
            fn deref_mut(&mut self) -> &mut Self::Target {
                assert!(self.root.is_none(), "published snapshots remain immutable in tests");
                Rc::get_mut(&mut self.data).expect("synthetic executable remains uniquely owned")
            }
        }''' if cached else """impl std::ops::DerefMut for PublishedFunctionSnapshot { fn deref_mut(&mut self) -> &mut Self::Target { assert!(self.root.is_none(), "published snapshots remain immutable in tests"); &mut self.data } }""")
    if not re.search(r"#\[cfg\(test\)\]\s+impl\s+std::ops::DerefMut\s+for\s+PublishedFunctionSnapshot", code):
        ctx.fail(rule, "mutable projection must remain test-only")
    # The exact cache constructor ties every projected field to the same
    # validated bytecode node; cached data owns no FunctionBytecodeRef cycle.
    require(ctx, rule, "snapshot must validate Runtime/realm before selecting the bytecode-owned Rc cache and root the same node",
        function(ctx, production, "snapshot_function_bytecode", rule), BORROWED_SNAPSHOT_FUNCTION if owned else (SNAPSHOT_FUNCTION if cached else DIRECT_SNAPSHOT_FUNCTION))
    if owned:
        require(ctx, rule, "consuming snapshot must validate Runtime/realm and retain the same node with immutable observation facts",
            function(ctx, production, "snapshot_function_bytecode_owned", rule), OWNED_SNAPSHOT_FUNCTION)


EXECUTABLE_FUNCTIONS = {
    "frame_layout": """pub(crate) fn frame_layout(&self) -> crate::engine::code::function::layout::FrameLayout<'_> {
        crate::engine::code::function::layout::FrameLayout::new(&self.metadata, &self.argument_definitions, &self.local_definitions, &self.closure_variables,)
    }""",
    "constant": """pub(crate) fn constant(&self, index: u32) -> Option<&BytecodeConstant> {
        usize::try_from(index).ok().and_then(|index| self.constants.get(index))
    }""",
    "same_environment": '''pub(crate) fn same_environment(&self, other: &Self) -> bool {
        self.index == other.index && Rc::ptr_eq(&self.environments, &other.environments)
    }''',
    "owner": '''pub(crate) fn owner(&self) -> &FunctionBytecodeRef { &self.owner }''',
    "root": '''pub(crate) fn root(&self) -> Option<&FunctionBytecodeRef> { self.root.as_ref() }''',
    "eval_environment": '''pub(crate) fn eval_environment(&self, index: u16) -> Option<PublishedEvalEnvironment> {
        let index = usize::from(index); self.eval_environments.get(index)?;
        Some(PublishedEvalEnvironment { owner: self.root.as_ref()?.clone(),
            environments: self.eval_environments.clone(), index, })
    }''',
}

# Reviewed cached constructor: exact field mapping and validation order.
SNAPSHOT_FUNCTION = """    pub(crate) fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let root = function.clone();
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        let data = bytecode.executable.get_or_init(|| {
            let data = Rc::new(PublishedFunctionData {
                #[cfg(feature = "stack-vm")]
                fusion: bytecode.fusion.clone(),
                code: bytecode.code.clone(),
                constants: bytecode.constants.clone(),
                property_key_atoms: bytecode.property_key_atoms.clone(),
                argument_definitions: bytecode.argument_definitions.clone(),
                local_definitions: bytecode.local_definitions.clone(),
                closure_variables: bytecode.closure_variables.clone(),
                eval_environments: bytecode.eval_environments.clone(),
                arg_eval_variable_object_local: bytecode
                    .parameter_environment
                    .as_ref()
                    .and_then(|layout| layout.arg_eval_variable_object_local),
                metadata: bytecode.metadata,
                realm: bytecode.realm,
            });
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_call_buffer_capacity(
                "executable.published_data_rc",
                0,
                1,
                size_of::<PublishedFunctionData>(),
            );
            data
        });
        let data = data.clone();
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_share(
            "executable.published_data_rc",
            1,
            size_of::<PublishedFunctionData>(),
        );

        Ok(PublishedFunctionSnapshot {
            root: Some(root),
            data,
        })
    }"""

# Immutable cache fields; no runtime root is retained by the bytecode-owned cache.
PUBLISHED_DATA = """pub(crate) struct PublishedFunctionData {
    #[cfg(feature = "stack-vm")]
    pub(crate) fusion: crate::engine::code::fusion::FusionPlan,
    pub(crate) code: Rc<[crate::engine::code::bytecode::Instruction]>,
    pub(crate) constants: Rc<[BytecodeConstant]>,
    pub(crate) property_key_atoms: Option<Rc<[Atom]>>,
    pub(crate) argument_definitions: Rc<[VariableDefinition]>,
    pub(crate) local_definitions: Rc<[VariableDefinition]>,
    pub(crate) closure_variables: Rc<[ClosureVariable]>,
    pub(crate) eval_environments: Rc<[EvalEnvironment<Atom>]>,
    /// Parameter-scope variable-object slot, carried separately from the
    /// body `<var>` slot in `FunctionMetadata`.
    pub(crate) arg_eval_variable_object_local: Option<u16>,
    pub(crate) metadata: FunctionMetadata,
    pub(crate) realm: ContextId,
}"""

# S08 direct projection authenticates identical fields/owner without the S09 Rc cache.
DIRECT_SNAPSHOT_FUNCTION = """    pub(crate) fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let root = function.clone();
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        Ok(PublishedFunctionSnapshot {
            root: Some(root),
            data: PublishedFunctionData {
                #[cfg(feature = "stack-vm")]
                fusion: bytecode.fusion.clone(),
                code: bytecode.code.clone(),
                constants: bytecode.constants.clone(),
                property_key_atoms: bytecode.property_key_atoms.clone(),
                argument_definitions: bytecode.argument_definitions.clone(),
                local_definitions: bytecode.local_definitions.clone(),
                closure_variables: bytecode.closure_variables.clone(),
                eval_environments: bytecode.eval_environments.clone(),
                arg_eval_variable_object_local: bytecode
                    .parameter_environment
                    .as_ref()
                    .and_then(|layout| layout.arg_eval_variable_object_local),
                metadata: bytecode.metadata,
                realm: bytecode.realm,
            },
        })
    }"""

BORROWED_SNAPSHOT_FUNCTION = """    pub(crate) fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        self.snapshot_function_bytecode_owned(function.clone())
    }"""

OWNED_SNAPSHOT_FUNCTION = """    pub(crate) fn snapshot_function_bytecode_owned(
        &self,
        function: FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        let data = bytecode.executable.get_or_init(|| {
            let data = Rc::new(PublishedFunctionData {
                observes_arguments: bytecode.code.iter().any(|op| {
                    matches!(
                        op,
                        crate::engine::code::bytecode::Instruction::Arguments(_)
                            | crate::engine::code::bytecode::Instruction::Rest(_)
                            | crate::engine::code::bytecode::Instruction::Eval { .. }
                            | crate::engine::code::bytecode::Instruction::ApplyEval { .. }
                    )
                }),
                #[cfg(feature = "stack-vm")]
                fusion: bytecode.fusion.clone(),
                code: bytecode.code.clone(),
                constants: bytecode.constants.clone(),
                property_key_atoms: bytecode.property_key_atoms.clone(),
                argument_definitions: bytecode.argument_definitions.clone(),
                local_definitions: bytecode.local_definitions.clone(),
                closure_variables: bytecode.closure_variables.clone(),
                eval_environments: bytecode.eval_environments.clone(),
                arg_eval_variable_object_local: bytecode
                    .parameter_environment
                    .as_ref()
                    .and_then(|layout| layout.arg_eval_variable_object_local),
                metadata: bytecode.metadata,
                realm: bytecode.realm,
            });
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_call_buffer_capacity(
                "executable.published_data_rc",
                0,
                1,
                size_of::<PublishedFunctionData>(),
            );
            data
        });
        let data = data.clone();
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_share(
            "executable.published_data_rc",
            1,
            size_of::<PublishedFunctionData>(),
        );

        Ok(PublishedFunctionSnapshot {
            root: Some(function),
            data,
        })
    }"""

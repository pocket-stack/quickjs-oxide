//! A generator's executing state belongs to its suspended native operation.
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::object::ObjectRef;
use crate::engine::vm::call::NativeInvokeOutcome;
use crate::engine::vm::suspend::{RootedVmActivation, VmActivationResume, VmRunOutcome};

pub(crate) enum GeneratorStep {
    Complete(NativeInvokeOutcome),
    Run {
        activation: Box<RootedVmActivation>,
        input: VmActivationResume,
        resume: Box<GeneratorResume>,
    },
}

impl GeneratorStep {
    pub(super) fn finish(self, runtime: &Runtime) -> Result<NativeInvokeOutcome, RuntimeError> {
        match self {
            Self::Complete(result) => Ok(result),
            Self::Run {
                activation,
                input,
                resume,
            } => {
                let outcome = activation.run(runtime, input)?;
                match resume.resume(outcome)? {
                    Self::Complete(result) => Ok(result),
                    Self::Run { .. } => Err(RuntimeError::Invariant(
                        "generator resume requested another frame",
                    )),
                }
            }
        }
    }
}

pub(crate) struct GeneratorResume {
    pub(super) runtime: Runtime,
    pub(super) generator: ObjectRef,
    pub(super) active: bool,
}

impl GeneratorResume {
    pub(crate) fn resume(
        mut self: Box<Self>,
        outcome: VmRunOutcome,
    ) -> Result<GeneratorStep, RuntimeError> {
        let result = self
            .runtime
            .finish_generator_resume(&self.generator, outcome)?;
        self.active = false;
        Ok(GeneratorStep::Complete(result))
    }
}

impl Drop for GeneratorResume {
    fn drop(&mut self) {
        if self.active {
            // Abandonment never publishes a resumable heap record. Invariant
            // errors and unwinding leave the branded object completed.
            let _ = self.runtime.complete_executing_generator(&self.generator);
        }
    }
}

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::{api::runtime::Runtime, value::Value};

    #[test]
    fn recursive_delegation_and_finally_use_one_owned_driver() {
        std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(|| {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let Value::Object(function) = context.eval("(function(){function* g(n){try{if(n) return yield* g(n-1); yield 42;}finally{if(!n)yield [1,2].map(x=>x+1)[1];}} var i=g(1000); var a=i.next();var b=i.return(9);var c=i.next();return a.value===42&&!a.done&&b.value===3&&!b.done&&c.value===9&&c.done;})").unwrap() else {panic!("expected function")};
            let callable = runtime.as_callable(&function).unwrap().unwrap();
            let profile = CostProfile::start();
            assert_eq!(context.call(&callable, Value::Undefined, &[]).unwrap(), Value::Bool(true));
            let cost = profile.snapshot();
            assert_eq!(cost.legacy_dispatches, 0, "{cost:?}");
            assert_eq!(cost.owned_bridge_exits, 0, "{cost:?}");
            assert_eq!(cost.owned_sync_call_bridges, 0, "{cost:?}");
        }).unwrap().join().unwrap();
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<GeneratorStep>() <= 64);

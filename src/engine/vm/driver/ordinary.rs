//! The ordinary call/return loop's only entry and destruction transactions.
use crate::engine::{
    api::{Error, runtime::Runtime},
    vm::{
        call::ordinary::DirectSelection,
        exception::runtime_error_to_vm_error,
        execution::RunningExecution,
        frame::{FrameId, ReturnValue},
    },
};

pub(super) enum Entry {
    Ordinary,
    Native(super::CallStep),
    NativeReady,
    General,
}

pub(super) fn enter(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    count: u16,
    method: bool,
    tail: bool,
) -> Result<Entry, Error> {
    enter_selected(runtime, execution, id, count, method, tail, None)
}

pub(super) fn enter_selected(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    count: u16,
    method: bool,
    tail: bool,
    selected_native: Option<crate::engine::object::LinkedNativeSelection>,
) -> Result<Entry, Error> {
    let logical_depth = execution.frames.logical_active_depth(runtime);
    let frame = execution.frames.current_mut(id)?;
    let count = usize::from(count);
    let depth = execution.slots.depth(&frame.window);
    let mut transaction = execution.slots.frame_transaction(&mut frame.window)?;
    transaction.peek(count + usize::from(method))?;
    enum Prepared {
        Ordinary(crate::engine::vm::call::ordinary::OrdinaryCall),
        Native(
            crate::engine::object::CallableRef,
            crate::engine::vm::frames::NativeClassification,
        ),
    }
    // End every Result/selection container holding a slot borrow before any
    // frame installation or operand transfer. Only owning facts leave here.
    let prepared = if let Some(selected) = selected_native {
        if !transaction.validate_call_value_domains(runtime, count, method)? {
            return Ok(Entry::General);
        }
        let Some((callable, selected)) =
            crate::engine::vm::frames::NativeClassification::promote_linked(
                selected,
                transaction.peek(count)?,
            )
        else {
            return Ok(Entry::General);
        };
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "native_linked_classification_consumed",
        );
        Prepared::Native(callable, selected)
    } else {
        let selection_result = DirectSelection::select(runtime, transaction.peek(count)?);
        if matches!(selection_result, Ok(DirectSelection::General)) {
            return Ok(Entry::General);
        }
        if !transaction.validate_call_value_domains(runtime, count, method)? {
            return Ok(Entry::General);
        }
        let selection = selection_result.map_err(runtime_error_to_vm_error)?;
        match selection {
            DirectSelection::Ordinary(ordinary) => Prepared::Ordinary(
                ordinary
                    .authenticate(runtime)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            DirectSelection::Native(native) => {
                let (callable, selected) =
                    crate::engine::vm::frames::NativeClassification::promote_selected(native);
                Prepared::Native(callable, selected)
            }
            DirectSelection::General => return Ok(Entry::General),
        }
    };
    match prepared {
        Prepared::Ordinary(call) => {
            drop(transaction);
            if !execution.frames.can_push() || runtime.bytecode_call_would_overflow() {
                return Ok(Entry::General);
            }
            call.install(runtime, execution, id, count, method, tail)?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            Ok(Entry::Ordinary)
        }
        Prepared::Native(callable, mut selected) => {
            let target = selected.target();
            let realm = selected.defining_realm();
            let minimum = selected.minimum();
            let operation = selected.take_operation();
            let (arguments, receiver) =
                transaction.take_native_call_operands(logical_depth, count, method)?;
            drop(transaction);
            execution.frames.materialize(runtime)?;
            let result = super::super::proxy_get_driver::start_native_with_classification(
                runtime,
                execution,
                id,
                callable,
                target,
                realm,
                minimum,
                receiver,
                arguments,
                tail,
                depth,
                Some(selected),
                operation,
            )?;
            if matches!(result, super::CallStep::Entered)
                && execution.frames.current_id() == Some(id)
                && !execution.frames.current_mut(id)?.cold.has_pending_query()
                && execution.pending.is_none()
            {
                Ok(Entry::NativeReady)
            } else {
                Ok(Entry::Native(result))
            }
        }
    }
}

pub(super) fn finish(execution: &mut RunningExecution, id: FrameId) -> Result<bool, Error> {
    let frame = execution.frames.current_mut(id)?;
    let Some(target) = frame.cold.ordinary_return() else {
        return Ok(false);
    };
    if execution.pending.is_none() {
        return Ok(false);
    }
    // Result ownership precedes window clearing and activation removal.
    let value = execution.pending.take().unwrap();
    let mut frame = execution.frames.pop(id)?;
    let guard = frame.cold.entry_guard.take();
    execution.slots.clear_frame(frame.window.take())?;
    if let Some(guard) = guard {
        guard.finish().map_err(runtime_error_to_vm_error)?;
    }
    execution.call_storage.recycle(frame.cold);
    let parent = execution.frames.current_mut(target.frame()?)?;
    if matches!(target.value_use, ReturnValue::Push) {
        execution.slots.push(&mut parent.window, value)?;
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event("ordinary_return_direct");
    Ok(true)
}

#[cfg(test)]
mod layout_tests {
    #[test]
    fn literal_method_and_native_ready_keep_receivers_errors_and_argument_order() {
        use crate::engine::api::{Runtime, Value};
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for source in [
            "Math.min(3, 2, 1) === 1 && Math.max() === -Infinity",
            "(()=>{let i=7;let r=Math.min(i,500);return r===7})()",
            "((i)=>{let r=Math.min(i,500);return r===7})(7)",
            "(()=>{let key={},value='text',m=new Map();m.set(key,value);return m.get(key)===value})()",
            "(()=>{let i=3;function capture(){return i}let r=Math.min(i,500);return r===capture()})()",
            "(()=>{try{Math.min(i,500);let i=3}catch(e){return e instanceof ReferenceError}return false})()",
            "(()=>{let i=2;let o={get m(){i=7;return Math.min}};let r=o.m(i,500);return r===7})()",
            "(()=>{let i=2;let o=new Proxy({m:Math.min},{get(t,k){i=9;return t[k]}});let r=o.m(i,500);return r===9})()",
            "(()=>{let n=0,a={valueOf(){n++;return 7}};let r=Math.min(a,500);return r===7&&n===1})()",
            "(()=>{let x=Symbol();try{Math.min(x,500)}catch(e){return e instanceof TypeError}return false})()",
            "(()=>{try{Math.min(Symbol())}catch(e){return e instanceof TypeError}return false})()",
            "Object.defineProperty(Math.min,'length',{value:99}); Math.min(2,3)===2",
            "(()=>{let o={x:9,m(a,b){return this.x+a+b}};return o.m(1,2)===12})()",
            "(()=>{let log='';let o={get m(){log+='g';return function(x){log+='c';return x}}};let x=o.m((log+='a',7));return x===7&&log==='gac'})()",
            "(()=>{let n=0;let o={get m(){n++;throw 8}};try{o.m(1,2)}catch(e){return e===8&&n===1}return false})()",
            "(()=>{let o={m:0};try{o.m(1,2)}catch(e){return e instanceof TypeError}return false})()",
            "(()=>{let m=new Map();m.set(1,2);return m.get(1)===2&&m.has(1)})()",
            "(()=>{let f=Math.min;Math.min=(a,b)=>a+b;let v=Math.min(2,3);Math.min=f;return v===5&&Math.min(2,3)===2})()",
        ] {
            assert_eq!(context.eval(source).unwrap(), Value::Bool(true), "{source}");
        }
        assert_eq!(runtime.0.active_frame_depth.get(), 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn variable_method_argument_consumes_linked_native_fact() {
        use crate::engine::api::{Runtime, Value, profiling::CostProfile};
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        // Resolve the lazy builtin once; the selected own-data path must be
        // exercised, while first-access autoinit keeps its canonical fallback.
        context.eval("Math.min").unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval("(()=>{let i=7;let result=Math.min(i,500);return result})()")
                .unwrap(),
            Value::Int(7)
        );
        let costs = profile.snapshot();
        assert_eq!(
            costs.owned_execution_events.get("method_call_span"),
            Some(&1)
        );
        assert_eq!(
            costs
                .owned_execution_events
                .get("native_linked_classification_consumed"),
            Some(&1)
        );
        assert_eq!(costs.legacy_dispatches, 0);
        assert_eq!(costs.owned_bridge_exits, 0);
    }

    #[test]
    fn unified_call_entry_keeps_the_ordinary_result_abi_size() {
        // Error already determines the old result's size. Adding the native
        // completion must not enlarge every ordinary Call return transaction.
        assert_eq!(
            std::mem::size_of::<Result<super::Entry, super::Error>>(),
            std::mem::size_of::<Result<bool, super::Error>>(),
        );
    }
}

//! Pure Number transactions inside an authenticated continuous window.
#[cfg(feature = "profiling")]
use super::{Cost, record_owned_storage};
use super::{Error, FrameBinding, FrameWindow, SlotStore};
use crate::engine::value::number::operations::Number;

impl SlotStore {
    pub(super) fn consume_number_pair_current(
        &mut self,
        window: &mut FrameWindow,
        operation: impl FnOnce(Number, Number) -> bool,
    ) -> Result<Option<bool>, Error> {
        let offset = window
            .depth
            .checked_sub(2)
            .ok_or_else(|| Error::internal("owned operand stack underflow"))?;
        let index = window.operands().start + offset;
        let [
            Some(FrameBinding::Direct(left)),
            Some(FrameBinding::Direct(right)),
        ] = &self.slots[index..index + 2]
        else {
            return Err(Error::internal("owned operand slot is not a value"));
        };
        let (Some(left), Some(right)) = (left.as_number_repr(), right.as_number_repr()) else {
            return Ok(None);
        };
        let result = operation(left, right);
        // Neither Number owner can trigger release, GC or a callback.
        self.slots[index] = None;
        self.slots[index + 1] = None;
        window.depth -= 2;
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= 2;
            record_owned_storage(Cost::Clear(2));
            crate::engine::api::profiling::record_owned_execution_event(
                "number_pair_consumed_in_place",
            );
        }
        Ok(Some(result))
    }

    pub(super) fn update_number_local_current(
        &mut self,
        window: &mut FrameWindow,
        index: u16,
        operation: impl FnOnce(Number) -> (Number, Option<Number>),
    ) -> Result<bool, Error> {
        let FrameBinding::Direct(previous) = self.local_current(window, index)? else {
            return Ok(false);
        };
        let Some(previous) = previous.as_number_repr() else {
            return Ok(false);
        };
        let (replacement, result) = operation(previous);
        // Preflight the optional result destination before modifying the local.
        // This also leaves malformed synthetic windows transactional on error.
        let result = if let Some(result) = result {
            if window.depth >= window.operands().len() {
                return Err(Error::internal(
                    "owned operand stack exceeds verified capacity",
                ));
            }
            let destination = window.operands().start + window.depth;
            if self.slots[destination].is_some() {
                return Err(Error::internal(
                    "owned operand push would replace a live value",
                ));
            }
            Some((destination, result))
        } else {
            None
        };
        self.slots[window.locals().start + usize::from(index)] =
            Some(FrameBinding::Direct(replacement.into()));
        if let Some((destination, result)) = result {
            self.slots[destination] = Some(FrameBinding::Direct(result.into()));
            window.depth += 1;
            #[cfg(feature = "profiling")]
            {
                self.live_slots += 1;
                record_owned_storage(Cost::Move(1));
                self.record_occupancy();
            }
        }
        #[cfg(feature = "profiling")]
        {
            record_owned_storage(Cost::Move(1));
            crate::engine::api::profiling::record_owned_execution_event(
                "number_local_updated_in_place",
            );
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::Runtime;
    use crate::engine::code::function::metadata::{ClosureVariableKind, VariableDefinition};
    use crate::engine::code::runtime::PublishedFunctionSnapshot;
    use crate::engine::value::Value;
    use crate::engine::vm::stack::FrameStorage;
    use std::rc::Rc;

    fn frame(runtime: &Runtime, capacity: u16) -> (SlotStore, FrameWindow) {
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.local_count = 1;
        owner.metadata.max_stack = capacity;
        owner.local_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        let mut slots = SlotStore::new(20);
        let window = slots
            .push_frame(
                &owner.frame_layout(),
                FrameStorage {
                    original_arguments: Vec::new(),
                    parameters: Vec::new(),
                    locals: vec![FrameBinding::Direct(Value::Int(7))],
                    operands: Vec::new(),
                },
            )
            .unwrap();
        (slots, window)
    }

    #[test]
    fn fused_pair_declines_before_mutation_and_clears_both_number_owners() {
        let runtime = Runtime::new();
        let (mut slots, mut window) = frame(&runtime, 2);
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        slots.push(&mut window, Value::Int(3)).unwrap();
        slots.push(&mut window, Value::Object(object)).unwrap();
        assert_eq!(
            slots
                .run_window(&mut window)
                .unwrap()
                .consume_number_pair(|_, _| {
                    panic!("non-number must decline before evaluating operation")
                })
                .unwrap(),
            None
        );
        assert_eq!(window.depth, 2);
        assert!(
            matches!(slots.peek(&window, 0).unwrap(), Value::Object(value) if value.object_id()==id)
        );
        drop(slots.pop(&mut window).unwrap());
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        slots.push(&mut window, Value::Int(7)).unwrap();
        assert_eq!(
            slots
                .run_window(&mut window)
                .unwrap()
                .consume_number_pair(|left, right| left.float() < right.float())
                .unwrap(),
            Some(true)
        );
        assert_eq!(window.depth, 0);
        assert!(slots.slots[window.operands()].iter().all(Option::is_none));
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .consume_number_pair(|_, _| true)
                .is_err()
        );
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn fused_local_result_capacity_failure_leaves_binding_and_stack_unchanged() {
        let runtime = Runtime::new();
        let (mut slots, mut window) = frame(&runtime, 1);
        slots.push(&mut window, Value::Int(99)).unwrap();
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .update_number_local(0, |previous| (previous.update(true), Some(previous)))
                .is_err()
        );
        assert!(matches!(
            slots.local(&window, 0).unwrap(),
            FrameBinding::Direct(Value::Int(7))
        ));
        assert_eq!(slots.peek(&window, 0).unwrap(), &Value::Int(99));
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .update_number_local(0, |previous| (previous.update(true), None))
                .unwrap()
        );
        assert!(matches!(
            slots.local(&window, 0).unwrap(),
            FrameBinding::Direct(Value::Int(8))
        ));
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(99));
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .update_number_local(0, |previous| (previous.update(false), Some(previous)))
                .unwrap()
        );
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(8));
        assert!(matches!(
            slots.local(&window, 0).unwrap(),
            FrameBinding::Direct(Value::Int(7))
        ));
        slots
            .replace_local(&window, 0, FrameBinding::Uninitialized)
            .unwrap();
        assert!(
            !slots
                .run_window(&mut window)
                .unwrap()
                .update_number_local(0, |_| panic!("TDZ must not evaluate"))
                .unwrap()
        );
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .update_number_local(1, |_| panic!("invalid local must not evaluate"))
                .is_err()
        );
        slots.clear_frame(window).unwrap();
    }
}

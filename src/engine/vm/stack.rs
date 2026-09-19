//! Exclusive frame windows in reusable owning storage.
//!
//! A window contains indices, never addresses into the arena. Growing another
//! window therefore cannot invalidate a retained frame identity. Empty slots
//! have no Value owner; TDZ is the distinct FrameBinding::Uninitialized state.

use crate::engine::api::error::Error;
#[cfg(feature = "profiling")]
use crate::engine::api::profiling::{OwnedStorageEvent as Cost, record_owned_storage};
use crate::engine::api::runtime::Runtime;
use crate::engine::code::function::layout::FrameLayout;
use crate::engine::value::{JsValue, Value};
use crate::engine::vm::bindings::{FrameBinding, release_frame_binding as release_binding};
use crate::engine::vm::exception::runtime_error_to_vm_error;
use std::ops::Range;
use std::rc::Rc;
mod call;

pub(in crate::engine::vm) struct SlotStore {
    // Initialized high-water backing. Inactive slots are always None; only
    // active_end participates in frame authority and the logical slot limit.
    slots: Vec<Option<FrameBinding>>,
    active_end: usize,
    // Outgoing synchronous call argument buffer (internal values).
    argument_buffer: Vec<JsValue>,
    // Native-boundary argv buffers: operands are rooted into the public
    // representation at the native dispatch boundary.
    native_argument_buffers: Vec<Vec<Value>>,
    owner: Rc<()>,
    next_window: u64,
    windows: Vec<u64>,
    limit: usize,
    #[cfg(feature = "profiling")]
    live_slots: usize,
    // Scratch carrier for the take-frame profiling omission count between the
    // span move and its completion; never observed across operations.
    #[cfg(feature = "profiling")]
    omitted: usize,
}

/// Not Clone: releasing a frame consumes its authority over the window.
pub(in crate::engine::vm) struct FrameWindow {
    actual_count: usize,
    owner: Rc<()>,
    id: u64,
    // Consecutive regions share their boundaries. Keep usize widths and
    // derive the same Range values rather than storing each boundary twice.
    base: usize,
    original_end: usize,
    parameters_end: usize,
    locals_end: usize,
    end: usize,
    depth: usize,
}

impl FrameWindow {
    #[inline]
    fn whole(&self) -> Range<usize> {
        self.base..self.end
    }

    #[inline]
    fn original_arguments(&self) -> Range<usize> {
        self.base..self.original_end
    }

    #[inline]
    fn parameters(&self) -> Range<usize> {
        self.original_end..self.parameters_end
    }

    #[inline]
    fn locals(&self) -> Range<usize> {
        self.parameters_end..self.locals_end
    }

    #[inline]
    fn operands(&self) -> Range<usize> {
        self.locals_end..self.end
    }
}

pub(in crate::engine::vm) struct FrameStorage {
    pub original_arguments: Vec<JsValue>,
    pub parameters: Vec<FrameBinding>,
    pub locals: Vec<FrameBinding>,
    pub operands: Vec<JsValue>,
}

mod number;
mod window;
pub(in crate::engine::vm) use window::{FrameTransaction, LinkedReadCompletion, RunSlots};

impl SlotStore {
    /// Commit a retained IC result only after output capacity and the receiver
    /// release proof have succeeded. Failure leaves the canonical operands.
    // The slot window, immutable site facts and selected native output are disjoint borrowed inputs to one transaction.
    #[allow(clippy::too_many_arguments)]
    fn property_ic_read_current(
        &mut self,
        window: &mut FrameWindow,
        runtime: &Runtime,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        pc: usize,
        key_index: u32,
        keep_receiver: bool,
        native: &mut Option<crate::engine::object::LinkedNativeSelection>,
    ) -> Result<bool, Error> {
        let output_index = if keep_receiver {
            // Canonical get can have effects before an output-capacity error;
            // declining here preserves that order without promoting a root.
            let Ok(index) = self.operand_push_index(window) else {
                return Ok(false);
            };
            Some(index)
        } else {
            None
        };
        let base = self.peek_current(window, 0)?;
        let value = match runtime.property_ic_read_fast(
            base,
            executable,
            pc,
            key_index,
            keep_receiver,
            native,
        ) {
            Some(value) => value,
            None => {
                let Some(value) = runtime
                    .try_property_ic_read_owned(
                        base,
                        executable,
                        pc,
                        key_index,
                        keep_receiver,
                        native,
                    )
                    .map_err(crate::engine::vm::exception::runtime_error_to_vm_error)?
                else {
                    return Ok(false);
                };
                value
            }
        };
        if let Some(index) = output_index {
            self.install_operand(window, index, value);
        } else {
            let index = window.operands().start + window.depth - 1;
            let base = self.slots[index].replace(FrameBinding::Direct(value));
            // No reentry or cleanup queue mutation intervenes between the
            // runtime proof and this non-final receiver release.
            if let FrameBinding::Direct(old) = base {
                runtime
                    .release_jsvalue(old)
                    .map_err(runtime_error_to_vm_error)?;
            }
            #[cfg(feature = "profiling")]
            record_owned_storage(Cost::Move(2));
        }
        Ok(true)
    }

    pub(in crate::engine::vm) fn new(limit: usize) -> Self {
        Self {
            slots: Vec::new(),
            active_end: 0,
            argument_buffer: Vec::new(),
            native_argument_buffers: Vec::new(),
            owner: Rc::new(()),
            next_window: 1,
            windows: Vec::new(),
            limit,
            #[cfg(feature = "profiling")]
            live_slots: 0,
            #[cfg(feature = "profiling")]
            omitted: 0,
        }
    }

    /// Pool metadata follows simultaneous active-frame depth. Cached buffers
    /// are empty; native activations keep their owners until frame cleanup.
    pub(in crate::engine::vm) fn reserve_native_argument_depth(
        &mut self,
        depth: usize,
    ) -> Result<(), Error> {
        if depth <= self.native_argument_buffers.capacity() {
            return Ok(());
        }
        let _before = self.native_argument_buffers.capacity();
        self.native_argument_buffers
            .try_reserve(depth.saturating_sub(self.native_argument_buffers.len()))
            .map_err(|_| Error::internal("native argument recycler allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "call.native_pool",
            _before,
            self.native_argument_buffers.capacity(),
            size_of::<Vec<Value>>(),
        );
        Ok(())
    }

    pub(in crate::engine::vm) fn take_native_argument_buffer(
        &mut self,
        count: usize,
    ) -> Result<Vec<Value>, Error> {
        let mut arguments = self.native_argument_buffers.pop().unwrap_or_default();
        debug_assert!(arguments.is_empty());
        let _before = arguments.capacity();
        arguments
            .try_reserve_exact(count)
            .map_err(|_| Error::internal("native call arguments allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "call.native_argv",
            _before,
            arguments.capacity(),
            size_of::<Value>(),
        );
        Ok(arguments)
    }

    /// Native argv is an owning tail transfer, authenticated once. The caller
    /// has already validated domains in receiver/left-to-right argument order.
    /// Allocation and slot checks precede consumption; then only infallible
    /// moves occur until the original callee-release boundary.
    pub(in crate::engine::vm) fn take_native_call_operands(
        &mut self,
        runtime: &Runtime,
        window: &mut FrameWindow,
        count: usize,
        method: bool,
    ) -> Result<(Vec<Value>, Value), Error> {
        self.check_current(window)?;
        self.take_native_call_operands_current(runtime, window, count, method)
    }

    fn take_native_call_operands_current(
        &mut self,
        runtime: &Runtime,
        window: &mut FrameWindow,
        count: usize,
        method: bool,
    ) -> Result<(Vec<Value>, Value), Error> {
        let mut arguments = self.take_native_argument_buffer(count)?;
        for offset in 0..count + 1 + usize::from(method) {
            self.peek_current(window, offset)?;
        }
        let start = window.operands().start + window.depth - count;
        for index in start..start + count {
            let Some(FrameBinding::Direct(value)) = self.slots[index].take() else {
                unreachable!("native operand transaction authenticated each slot")
            };
            // Native dispatch consumes public roots; root the moved owner at
            // this boundary and release its internal edge.
            arguments.push(runtime.root_value(&value).map_err(runtime_error_to_vm_error)?);
            runtime
                .release_jsvalue(value)
                .map_err(runtime_error_to_vm_error)?;
        }
        window.depth -= count;
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= count;
            record_owned_storage(Cost::Move(count));
            crate::engine::api::profiling::record_owned_execution_event(
                "native_argv_transferred_in_order",
            );
            crate::engine::api::profiling::record_call_buffer_moves("call.native_argv", count);
        }
        // Match the previous callee then receiver pop/drop order. The classified
        // callable owner pins the callee throughout this transfer.
        let callee = self.pop_current(window)?;
        runtime
            .release_jsvalue(callee)
            .map_err(runtime_error_to_vm_error)?;
        let receiver = if method {
            let receiver = self.pop_current(window)?;
            let rooted = runtime.root_value(&receiver).map_err(runtime_error_to_vm_error)?;
            runtime.release_jsvalue(receiver).map_err(runtime_error_to_vm_error)?;
            rooted
        } else {
            Value::Undefined
        };
        Ok((arguments, receiver))
    }

    /// Cleanup cannot allocate or retain JavaScript owners. Producers outside
    /// the native caller path may supply extra buffers; discard excess capacity.
    pub(in crate::engine::vm) fn recycle_native_argument_buffer(&mut self, arguments: Vec<Value>) {
        debug_assert!(arguments.is_empty());
        if !arguments.is_empty() {
            return;
        }
        if self.native_argument_buffers.len() < self.native_argument_buffers.capacity() {
            self.native_argument_buffers.push(arguments);
        }
    }

    /// One empty outgoing buffer suffices for synchronous frame installation:
    /// the owners are transferred into slots before the child starts running.
    pub(in crate::engine::vm) fn take_argument_buffer(
        &mut self,
        count: usize,
    ) -> Result<Vec<JsValue>, Error> {
        let mut arguments = std::mem::take(&mut self.argument_buffer);
        debug_assert!(arguments.is_empty());
        let before = arguments.capacity();
        arguments
            .try_reserve(count)
            .map_err(|_| Error::internal("call arguments allocation failed"))?;
        #[cfg(feature = "profiling")]
        if arguments.capacity() > before {
            crate::engine::api::profiling::record_owned_execution_event(
                "call_outgoing_buffer_capacity_growth",
            );
        }
        #[cfg(not(feature = "profiling"))]
        let _ = before;
        Ok(arguments)
    }

    /// The exclusive borrow prevents arena growth, frame changes and any window
    /// reuse until run returns to its observation boundary.
    pub(in crate::engine::vm) fn run_window<'a>(
        &'a mut self,
        window: &'a mut FrameWindow,
    ) -> Result<RunSlots<'a>, Error> {
        self.check_current(window)?;
        Ok(RunSlots {
            store: self,
            window,
        })
    }

    /// All capacity checks precede ownership installation. Arguments already
    /// contain the writable, padded parameter bindings; the immutable original
    /// snapshot remains a separate range even for strict/non-simple parameters.
    pub(in crate::engine::vm) fn push_frame(
        &mut self,
        runtime: &Runtime,
        layout: &FrameLayout<'_>,
        storage: FrameStorage,
    ) -> Result<FrameWindow, Error> {
        self.push_frame_storage(runtime, layout, storage, None, None)
    }

    pub(in crate::engine::vm) fn push_initialized_frame(
        &mut self,
        runtime: &Runtime,
        layout: &FrameLayout<'_>,
        storage: FrameStorage,
        function: &crate::engine::object::ObjectRef,
        function_name: Option<u16>,
    ) -> Result<FrameWindow, Error> {
        self.push_frame_storage(runtime, layout, storage, Some((function, function_name)), None)
    }

    /// Reserve and copy the writable parameter snapshot before consuming any
    /// caller owner. Originals then move straight from the outgoing operand
    /// tail into the callee snapshot; the caller prefix never moves.
    pub(in crate::engine::vm) fn push_call_frame(
        &mut self,
        runtime: &Runtime,
        layout: &FrameLayout<'_>,
        parent: &mut FrameWindow,
        count: usize,
        method: bool,
        function: &crate::engine::object::ObjectRef,
        function_name: Option<u16>,
    ) -> Result<FrameWindow, Error> {
        self.check_current(parent)?;
        let consumed = count
            .checked_add(1 + usize::from(method))
            .filter(|consumed| *consumed <= parent.depth)
            .ok_or_else(|| Error::internal("outgoing call exceeds caller operands"))?;
        for offset in 0..consumed {
            self.peek_current(parent, offset)?;
        }
        self.push_frame_storage(
            runtime,
            layout,
            FrameStorage {
                original_arguments: Vec::new(),
                parameters: Vec::new(),
                locals: Vec::new(),
                operands: Vec::new(),
            },
            Some((function, function_name)),
            Some((parent, count, method)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn push_frame_storage(
        &mut self,
        runtime: &Runtime,
        layout: &FrameLayout<'_>,
        mut storage: FrameStorage,
        initialize: Option<(&crate::engine::object::ObjectRef, Option<u16>)>,
        source: Option<(&mut FrameWindow, usize, bool)>,
    ) -> Result<FrameWindow, Error> {
        let fresh = initialize.is_some();
        let actual_count = source
            .as_ref()
            .map_or(storage.original_arguments.len(), |(_, count, _)| *count);
        let source_start = source
            .as_ref()
            .map(|(parent, count, _)| parent.operands().start + parent.depth - count);
        let parameter_count = layout.argument_slots(actual_count);
        let local_count = layout.locals().len();
        if fresh
            && (!storage.parameters.is_empty()
                || !storage.locals.is_empty()
                || !storage.operands.is_empty())
        {
            return Err(Error::internal(
                "fresh frame already contains initialized bindings",
            ));
        }
        if initialize
            .is_some_and(|(_, name)| name.is_some_and(|index| usize::from(index) >= local_count))
        {
            return Err(Error::internal("function-name local is outside the frame"));
        }
        if (!fresh
            && (storage.parameters.len() != parameter_count || storage.locals.len() != local_count))
            || storage.operands.len() > layout.operand_capacity()
        {
            return Err(Error::internal(
                "owned frame storage disagrees with its published layout",
            ));
        }
        let next_window = self
            .next_window
            .checked_add(1)
            .ok_or_else(|| Error::internal("frame window identity exhausted"))?;
        let base = self.active_end;
        let original_end = base.checked_add(actual_count);
        let parameters_end = original_end.and_then(|n| n.checked_add(parameter_count));
        let locals_end = parameters_end.and_then(|n| n.checked_add(local_count));
        let end = locals_end
            .and_then(|n| n.checked_add(layout.operand_capacity()))
            .filter(|end| *end <= self.limit)
            .ok_or_else(|| Error::internal("execution slot limit exceeded"))?;
        #[cfg(feature = "profiling")]
        let capacity_before = self.slots.capacity();
        self.slots
            .try_reserve(end.saturating_sub(self.slots.len()))
            .map_err(|_| Error::internal("execution slot allocation failed"))?;
        #[cfg(feature = "profiling")]
        record_owned_storage(Cost::SlotCapacity {
            before: capacity_before,
            after: self.slots.capacity(),
        });
        self.windows
            .try_reserve(1)
            .map_err(|_| Error::internal("execution window allocation failed"))?;
        let original_end = original_end.unwrap();
        let parameters_end = parameters_end.unwrap();
        let locals_end = locals_end.unwrap();
        let depth = storage.operands.len();
        #[cfg(feature = "profiling")]
        let initialized_before = self.slots.len();
        if end > self.slots.len() {
            self.slots.resize_with(end, || None);
        }
        #[cfg(feature = "profiling")]
        record_owned_storage(Cost::NoneInitialization {
            count: self.slots.len() - initialized_before,
            high_water: self.slots.len(),
        });
        debug_assert!(self.slots[base..end].iter().all(Option::is_none));
        #[cfg(feature = "profiling")]
        let installed = actual_count + parameter_count + local_count + depth;
        if let Some((function, function_name)) = initialize {
            // Copy fallible roots before consuming the source snapshot. Roll
            // back this unpublished suffix on failure; parent owners stay put.
            #[cfg(feature = "profiling")]
            let mut root_copies = 0;
            for index in 0..actual_count {
                let value = if let Some(start) = source_start {
                    let Some(FrameBinding::Direct(value)) = &self.slots[start + index] else {
                        self.clear_unpublished(runtime, original_end..original_end + index)?;
                        return Err(Error::internal("outgoing argument is not a direct owner"));
                    };
                    value
                } else {
                    &storage.original_arguments[index]
                };
                #[cfg(feature = "profiling")]
                {
                    root_copies +=
                        usize::from(matches!(value, JsValue::Object(_) | JsValue::Symbol(_)));
                }
                match runtime.dup_jsvalue(value).map_err(runtime_error_to_vm_error) {
                    Ok(value) => {
                        self.slots[original_end + index] = Some(FrameBinding::Direct(value))
                    }
                    Err(error) => {
                        self.clear_unpublished(runtime, original_end..original_end + index)?;
                        return Err(error);
                    }
                }
            }
            for index in original_end + actual_count..parameters_end {
                self.slots[index] = Some(FrameBinding::Direct(JsValue::Undefined));
            }
            for (index, definition) in layout.locals().iter().enumerate() {
                let binding = super::call::prepare::initial_local_binding(
                    runtime,
                    definition.is_lexical,
                    function_name == Some(index as u16),
                    function,
                )
                .map_err(runtime_error_to_vm_error)?;
                self.slots[parameters_end + index] = Some(binding);
            }
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_call_preparation(
                parameter_count,
                0,
                local_count,
                0,
                actual_count,
                root_copies,
                1 + usize::from(function_name.is_some()),
            );
            if let Some((parent, count, method)) = source {
                let start = source_start.unwrap();
                for index in 0..count {
                    self.slots[base + index] = self.slots[start + index].take();
                }
                let consumed = count + 1 + usize::from(method);
                for index in start - 1 - usize::from(method)..start {
                    self.slots[index].take();
                }
                parent.depth -= consumed;
                #[cfg(feature = "profiling")]
                {
                    self.live_slots -= consumed;
                    record_owned_storage(Cost::Clear(consumed - count));
                    crate::engine::api::profiling::record_owned_execution_event(
                        "call_outgoing_tail_transferred",
                    );
                }
            } else {
                for (index, value) in storage.original_arguments.drain(..).enumerate() {
                    self.slots[base + index] = Some(FrameBinding::Direct(value));
                }
                if storage.original_arguments.capacity() > self.argument_buffer.capacity() {
                    self.argument_buffer = storage.original_arguments;
                }
            }
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "call_bindings_initialized_in_window",
            );
        } else {
            for (index, value) in storage.original_arguments.into_iter().enumerate() {
                self.slots[base + index] = Some(FrameBinding::Direct(value));
            }
            for (index, binding) in storage.parameters.into_iter().enumerate() {
                self.slots[original_end + index] = Some(binding);
            }
            for (index, binding) in storage.locals.into_iter().enumerate() {
                self.slots[parameters_end + index] = Some(binding);
            }
        }
        for (index, value) in storage.operands.into_iter().enumerate() {
            self.slots[locals_end + index] = Some(FrameBinding::Direct(value));
        }
        self.active_end = end;
        let id = self.next_window;
        self.next_window = next_window;
        self.windows.push(id);
        #[cfg(feature = "profiling")]
        {
            self.live_slots += installed;
            record_owned_storage(Cost::Initialize(end - base));
            record_owned_storage(Cost::Move(installed));
            self.record_occupancy();
        }
        Ok(FrameWindow {
            actual_count,
            owner: self.owner.clone(),
            id,
            base,
            original_end,
            parameters_end,
            locals_end,
            end,
            depth,
        })
    }

    // Only fallible parameter copies can reach this unpublished rollback.
    // Keep the backing initialized while releasing staged owners in index order.
    fn clear_unpublished(&mut self, runtime: &Runtime, range: Range<usize>) -> Result<(), Error> {
        for index in range {
            if let Some(binding) = self.slots[index].take() {
                release_binding(runtime, binding)?;
            }
        }
        Ok(())
    }

    #[cfg(feature = "profiling")]
    fn record_occupancy(&self) {
        record_owned_storage(Cost::Occupancy {
            reserved: self.active_end,
            live: self.live_slots,
        });
    }

    fn check_current(&self, window: &FrameWindow) -> Result<(), Error> {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("slot_authentication");
        if !Rc::ptr_eq(&self.owner, &window.owner)
            || self.windows.last() != Some(&window.id)
            || self.active_end != window.whole().end
        {
            return Err(Error::internal(
                "frame window is not the active arena window",
            ));
        }
        Ok(())
    }

    pub(in crate::engine::vm) fn depth(&self, window: &FrameWindow) -> usize {
        window.depth
    }

    pub(in crate::engine::vm) fn peek(
        &self,
        window: &FrameWindow,
        from_top: usize,
    ) -> Result<&JsValue, Error> {
        self.check_current(window)?;
        self.peek_current(window, from_top)
    }

    #[inline]
    fn peek_current(&self, window: &FrameWindow, from_top: usize) -> Result<&JsValue, Error> {
        let offset = from_top
            .checked_add(1)
            .and_then(|offset| window.depth.checked_sub(offset))
            .ok_or_else(|| Error::internal("owned operand stack underflow"))?;
        match &self.slots[window.operands().start + offset] {
            Some(FrameBinding::Direct(value)) => Ok(value),
            _ => Err(Error::internal("owned operand slot is not a value")),
        }
    }

    /// Validate only the receiver and argument domains, in the original call
    /// rejection order. Authentication is shared; each value is still read at
    /// its original position so a later malformed slot cannot mask an earlier
    /// foreign-domain value. This borrows no runtime state and moves no owner.
    pub(in crate::engine::vm) fn validate_call_value_domains(
        &self,
        window: &FrameWindow,
        runtime: &Runtime,
        count: usize,
        method: bool,
    ) -> Result<bool, Error> {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("call_value_domain_validation");
        // An ordinary zero-argument call had no domain-check reads at all.
        if count == 0 && !method {
            return Ok(true);
        }
        self.check_current(window)?;
        self.validate_call_value_domains_current(window, runtime, count, method)
    }

    fn validate_call_value_domains_current(
        &self,
        window: &FrameWindow,
        runtime: &Runtime,
        count: usize,
        method: bool,
    ) -> Result<bool, Error> {
        // Internal values carry no runtime branding: every operand domain is
        // inherently local. The peek order still authenticates each slot in
        // the original receiver/left-to-right rejection order.
        let _ = runtime;
        if method {
            self.peek_current(
                window,
                count
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("owned operand stack underflow"))?,
            )?;
        }
        for offset in (0..count).rev() {
            self.peek_current(window, offset)?;
        }
        Ok(true)
    }

    /// Number operands have no release effects. Authenticate the window and both
    /// operands before replacing either owner; a non-number leaves the stack intact.
    #[cfg(test)]
    pub(in crate::engine::vm) fn binary_number(
        &mut self,
        window: &mut FrameWindow,
        operation: impl FnOnce(
            crate::engine::value::number::operations::Number,
            crate::engine::value::number::operations::Number,
        ) -> JsValue,
    ) -> Result<bool, Error> {
        self.check_current(window)?;
        self.binary_number_current(window, operation)
    }

    #[inline]
    fn binary_number_current(
        &mut self,
        window: &mut FrameWindow,
        operation: impl FnOnce(
            crate::engine::value::number::operations::Number,
            crate::engine::value::number::operations::Number,
        ) -> JsValue,
    ) -> Result<bool, Error> {
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
            return Ok(false);
        };
        let result = operation(left, right);
        self.slots[index] = Some(FrameBinding::Direct(result));
        self.slots[index + 1] = None;
        window.depth -= 1;
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= 1;
            // One result owner enters storage; neither numeric input is moved out.
            record_owned_storage(Cost::Move(1));
            crate::engine::api::profiling::record_owned_execution_event("binary_number_in_place");
        }
        Ok(true)
    }

    fn typed_array_number_write_current(
        &mut self,
        window: &mut FrameWindow,
        runtime: &Runtime,
    ) -> Result<bool, Error> {
        let offset = window
            .depth
            .checked_sub(3)
            .ok_or_else(|| Error::internal("owned operand stack underflow"))?;
        let index = window.operands().start + offset;
        let [
            Some(FrameBinding::Direct(base)),
            Some(FrameBinding::Direct(key)),
            Some(FrameBinding::Direct(value)),
        ] = &self.slots[index..index + 3]
        else {
            return Err(Error::internal("owned operand slot is not a value"));
        };
        let JsValue::Int(key) = key else {
            return Ok(false);
        };
        if *key < 0 {
            return Ok(false);
        }
        let typed = match value {
            JsValue::Int(value) => {
                runtime.try_typed_array_number_write(base, *key as u32, f64::from(*value))
            }
            JsValue::Float(value) => runtime.try_typed_array_number_write(base, *key as u32, *value),
            _ => false,
        };
        if !typed
            && !runtime
                .try_dense_array_write_scalar(base, *key as u32, value)
                .map_err(super::exception::runtime_error_to_vm_error)?
        {
            return Ok(false);
        }
        // The successful leaf proved base's sole release cannot drain. Only
        // numeric input moves occur before its release; no proof can change.
        let value = self.slots[index + 2].take();
        let key = self.slots[index + 1].take();
        let base = self.slots[index].take();
        window.depth = offset;
        for binding in [value, key, base] {
            if let Some(binding) = binding {
                release_binding(runtime, binding)?;
            }
        }
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= 3;
            record_owned_storage(Cost::Move(3));
            crate::engine::api::profiling::record_owned_execution_event(if typed {
                "typed_array_number_write_in_run"
            } else {
                "dense_array_scalar_write_in_run"
            });
        }
        Ok(true)
    }

    fn array_immediate_read_current(
        &mut self,
        window: &mut FrameWindow,
        runtime: &Runtime,
    ) -> Result<bool, Error> {
        let offset = window
            .depth
            .checked_sub(2)
            .ok_or_else(|| Error::internal("owned operand stack underflow"))?;
        let index = window.operands().start + offset;
        let [
            Some(FrameBinding::Direct(base)),
            Some(FrameBinding::Direct(key)),
        ] = &self.slots[index..index + 2]
        else {
            return Err(Error::internal("owned operand slot is not a value"));
        };
        let index_key = match key {
            JsValue::Int(key) if *key >= 0 => *key as u32,
            JsValue::String(id) => {
                // Mirror the historical guard: only a key whose node survives
                // its own release takes this leaf. The spelling is cloned out
                // before the slot owners are consumed below.
                if !matches!(
                    runtime.slot_value_release_readiness_jsvalue(key),
                    Ok(crate::engine::heap::SlotReleaseReadiness::Ready)
                ) {
                    return Ok(false);
                }
                let text = runtime.0.state.borrow().heap.string_fast(*id).clone();
                let Some(index) = crate::engine::atom::AtomTable::canonical_array_index(&text)
                else {
                    return Ok(false);
                };
                index
            }
            _ => return Ok(false),
        };
        let Some(value) = runtime.try_array_immediate_read(base, index_key) else {
            return Ok(false);
        };
        // The scalar result owns no heap edge. Preflight proved that releasing
        // the base cannot drain; no ownership decrease intervened since then.
        let key = self.slots[index + 1].take();
        let base = self.slots[index].replace(FrameBinding::Direct(value));
        window.depth -= 1;
        for binding in [key, base] {
            if let Some(binding) = binding {
                release_binding(runtime, binding)?;
            }
        }
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= 1;
            record_owned_storage(Cost::Move(3));
            crate::engine::api::profiling::record_owned_execution_event(
                "array_immediate_read_in_run",
            );
        }
        Ok(true)
    }

    fn ordinary_field_immediate_read_current(
        &mut self,
        window: &mut FrameWindow,
        runtime: &Runtime,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        key_index: u32,
    ) -> Result<bool, Error> {
        let base = self.peek_current(window, 0)?;
        let Some(value) = runtime.try_ordinary_field_immediate_read(base, executable, key_index)
        else {
            return Ok(false);
        };
        // No owner or runtime state can change between the leaf's no-drain
        // proof and replacing this already-validated top operand.
        let index = window.operands().start + window.depth - 1;
        let base = self.slots[index].replace(FrameBinding::Direct(value));
        if let Some(binding) = base {
            release_binding(runtime, binding)?;
        }
        #[cfg(feature = "profiling")]
        {
            record_owned_storage(Cost::Move(2));
            crate::engine::api::profiling::record_owned_execution_event(
                "ordinary_field_immediate_read_in_run",
            );
        }
        Ok(true)
    }

    fn property_ic_write_scalar_current(
        &mut self,
        window: &mut FrameWindow,
        runtime: &Runtime,
        executable: &crate::engine::code::runtime::PublishedFunctionSnapshot,
        pc: usize,
        key: u32,
    ) -> Result<bool, Error> {
        let offset = window
            .depth
            .checked_sub(2)
            .ok_or_else(|| Error::internal("owned operand stack underflow"))?;
        let index = window.operands().start + offset;
        let [
            Some(FrameBinding::Direct(base)),
            Some(FrameBinding::Direct(value)),
        ] = &self.slots[index..index + 2]
        else {
            return Err(Error::internal("owned operand slot is not a value"));
        };
        if !runtime
            .try_property_ic_write_scalar(base, executable, pc, key, value)
            .map_err(super::exception::runtime_error_to_vm_error)?
        {
            return Ok(false);
        }
        let base = self.slots[index].take();
        let value = self.slots[index + 1].take();
        window.depth = offset;
        for binding in [base, value] {
            if let Some(binding) = binding {
                release_binding(runtime, binding)?;
            }
        }
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= 2;
            record_owned_storage(Cost::Move(2));
        }
        Ok(true)
    }

    /// Move an owned value into an already reserved, empty operand slot.
    pub(in crate::engine::vm) fn push(
        &mut self,
        window: &mut FrameWindow,
        value: JsValue,
    ) -> Result<(), Error> {
        self.check_current(window)?;
        self.push_current(window, value)
    }

    #[inline]
    fn push_current(&mut self, window: &mut FrameWindow, value: JsValue) -> Result<(), Error> {
        let index = self.operand_push_index(window)?;
        self.install_operand(window, index, value);
        Ok(())
    }

    #[inline]
    fn push_pending_current(
        &mut self,
        window: &mut FrameWindow,
        value: &mut Option<JsValue>,
    ) -> Result<(), Error> {
        let index = self.operand_push_index(window)?;
        self.install_operand(window, index, value.take().expect("pending operand owner"));
        Ok(())
    }

    /// Check before consuming an owner. Pending callers retain their value on
    /// rejection; ordinary pushes need no temporary Option or drop protocol.
    #[inline]
    fn operand_push_index(&self, window: &FrameWindow) -> Result<usize, Error> {
        if window.depth >= window.operands().len() {
            return Err(Error::internal(
                "owned operand stack exceeds verified capacity",
            ));
        }
        let index = window.operands().start + window.depth;
        if self.slots[index].is_some() {
            return Err(Error::internal(
                "owned operand push would replace a live value",
            ));
        }
        Ok(index)
    }

    /// The checked index is private and consumed without an observable boundary.
    #[inline]
    fn install_operand(&mut self, window: &mut FrameWindow, index: usize, value: JsValue) {
        self.slots[index] = Some(FrameBinding::Direct(value));
        window.depth += 1;
        #[cfg(feature = "profiling")]
        {
            self.live_slots += 1;
            record_owned_storage(Cost::Move(1));
            self.record_occupancy();
        }
    }

    /// Rotate existing owners in place. No retain, release, or allocation occurs.
    #[cfg(test)]
    pub(in crate::engine::vm) fn rotate_operands(
        &mut self,
        window: &FrameWindow,
        skip_top: usize,
        count: usize,
        left: bool,
    ) -> Result<(), Error> {
        self.check_current(window)?;
        self.rotate_operands_current(window, skip_top, count, left)
    }

    fn rotate_operands_current(
        &mut self,
        window: &FrameWindow,
        skip_top: usize,
        count: usize,
        left: bool,
    ) -> Result<(), Error> {
        let extent = skip_top
            .checked_add(count)
            .filter(|_| count > 0)
            .ok_or_else(|| Error::internal("invalid owned operand rotation"))?;
        self.peek_current(window, extent - 1)?;
        let end = window.operands().start + window.depth - skip_top;
        #[cfg(feature = "profiling")]
        if count > 1 {
            record_owned_storage(Cost::Move(count));
        }
        let values = &mut self.slots[end - count..end];
        if left {
            values.rotate_left(1);
        } else {
            values.rotate_right(1);
        }
        Ok(())
    }

    /// Copy once, then install by moving owners inside the reserved window.
    /// All shape/capacity checks and the fallible retain precede mutation.
    #[cfg(test)]
    pub(in crate::engine::vm) fn insert_copy(
        &mut self,
        runtime: &Runtime,
        window: &mut FrameWindow,
        source_from_top: usize,
        destination_from_top: usize,
    ) -> Result<(), Error> {
        self.check_current(window)?;
        self.insert_copy_current(runtime, window, source_from_top, destination_from_top)
    }

    fn insert_copy_current(
        &mut self,
        runtime: &Runtime,
        window: &mut FrameWindow,
        source_from_top: usize,
        destination_from_top: usize,
    ) -> Result<(), Error> {
        self.peek_current(window, source_from_top)?;
        if destination_from_top > window.depth || window.depth >= window.operands().len() {
            return Err(Error::internal("owned insertion exceeds verified capacity"));
        }
        let copied = copy_value(runtime, self.peek_current(window, source_from_top)?)?;
        self.push_current(window, copied)?;
        self.rotate_operands_current(window, 0, destination_from_top + 1, false)
    }

    /// Retain a sequence into reserved slots. If a retain fails, the committed
    /// prefix stays inside the logical window for the driver's error cleanup.
    /// This error is terminal, never a bridge/retry: no local rollback may drop
    /// roots and unexpectedly drain deferred releases inside the run loop.
    #[cfg(test)]
    pub(in crate::engine::vm) fn duplicate_operands(
        &mut self,
        runtime: &Runtime,
        window: &mut FrameWindow,
        count: usize,
    ) -> Result<(), Error> {
        self.check_current(window)?;
        self.duplicate_operands_current(runtime, window, count)
    }

    fn duplicate_operands_current(
        &mut self,
        runtime: &Runtime,
        window: &mut FrameWindow,
        count: usize,
    ) -> Result<(), Error> {
        let source = count
            .checked_sub(1)
            .ok_or_else(|| Error::internal("empty owned operand duplication"))?;
        self.peek_current(window, source)?;
        if count > window.operands().len() - window.depth {
            return Err(Error::internal(
                "owned duplication exceeds verified capacity",
            ));
        }
        for _ in 0..count {
            // As depth grows, this fixed offset visits the next original slot.
            let copied = copy_value(runtime, self.peek_current(window, source)?)?;
            self.push_current(window, copied)?;
        }
        Ok(())
    }

    /// Logical pop removes ownership immediately. No dead value survives above sp.
    pub(in crate::engine::vm) fn pop(&mut self, window: &mut FrameWindow) -> Result<JsValue, Error> {
        self.check_current(window)?;
        self.pop_current(window)
    }

    #[inline]
    fn pop_current(&mut self, window: &mut FrameWindow) -> Result<JsValue, Error> {
        self.peek_current(window, 0)?;
        window.depth -= 1;
        #[cfg(feature = "profiling")]
        {
            self.live_slots -= 1;
            record_owned_storage(Cost::Move(1));
        }
        let Some(FrameBinding::Direct(value)) =
            self.slots[window.operands().start + window.depth].take()
        else {
            unreachable!("peek authenticated this slot before the move")
        };
        Ok(value)
    }

    /// Replace an authenticated live operand without changing its depth or neighbors.
    /// The displaced owner is returned; the caller releases or moves it.
    pub(in crate::engine::vm) fn replace_operand(
        &mut self,
        window: &FrameWindow,
        from_top: usize,
        value: JsValue,
    ) -> Result<JsValue, Error> {
        self.peek(window, from_top)?;
        let index = window.operands().start + window.depth - from_top - 1;
        #[cfg(feature = "profiling")]
        record_owned_storage(Cost::Move(2));
        let Some(FrameBinding::Direct(previous)) =
            self.slots[index].replace(FrameBinding::Direct(value))
        else {
            unreachable!()
        };
        Ok(previous)
    }

    /// Preflight and release one operand without moving any other owner. A
    /// false result leaves both the value and logical depth untouched.
    #[cfg(test)]
    pub(in crate::engine::vm) fn release_operand(
        &mut self,
        window: &FrameWindow,
        from_top: usize,
        runtime: &Runtime,
    ) -> Result<bool, Error> {
        self.check_current(window)?;
        self.release_operand_current(window, from_top, runtime)
    }

    fn release_operand_current(
        &mut self,
        window: &FrameWindow,
        from_top: usize,
        runtime: &Runtime,
    ) -> Result<bool, Error> {
        self.peek_current(window, from_top)?;
        let index = window.operands().start + window.depth - from_top - 1;
        let Some(FrameBinding::Direct(value)) = &mut self.slots[index] else {
            unreachable!()
        };
        runtime
            .try_release_slot_value_jsvalue(value)
            .map_err(runtime_error_to_vm_error)
    }

    pub(super) fn binding_counts(&self, window: &FrameWindow) -> Result<(usize, usize), Error> {
        self.check_current(window)?;
        Ok((window.locals().len(), window.parameters().len()))
    }

    pub(in crate::engine::vm) fn local(
        &self,
        window: &FrameWindow,
        index: u16,
    ) -> Result<&FrameBinding, Error> {
        self.check_current(window)?;
        self.local_current(window, index)
    }

    #[inline]
    fn local_current(&self, window: &FrameWindow, index: u16) -> Result<&FrameBinding, Error> {
        if usize::from(index) >= window.locals().len() {
            return Err(Error::internal("owned local index is out of bounds"));
        }
        self.slots[window.locals().start + usize::from(index)]
            .as_ref()
            .ok_or_else(|| Error::internal("owned local is vacant"))
    }

    /// The displaced owner is returned for explicit release at the caller's
    /// observation boundary. Installing the replacement never drops it implicitly.
    /// Borrow only for a NoJS binding operation; never retain across frame pushes.
    pub(in crate::engine::vm) fn local_mut(
        &mut self,
        window: &FrameWindow,
        index: u16,
    ) -> Result<&mut FrameBinding, Error> {
        self.local(window, index)?;
        Ok(self.slots[window.locals().start + usize::from(index)]
            .as_mut()
            .unwrap())
    }

    pub(in crate::engine::vm) fn parameter_mut(
        &mut self,
        window: &FrameWindow,
        index: u16,
    ) -> Result<&mut FrameBinding, Error> {
        self.parameter(window, index)?;
        Ok(self.slots[window.parameters().start + usize::from(index)]
            .as_mut()
            .unwrap())
    }

    pub(in crate::engine::vm) fn replace_local(
        &mut self,
        window: &FrameWindow,
        index: u16,
        value: FrameBinding,
    ) -> Result<FrameBinding, Error> {
        self.check_current(window)?;
        self.replace_local_current(window, index, value)
    }

    #[inline]
    fn replace_local_current(
        &mut self,
        window: &FrameWindow,
        index: u16,
        value: FrameBinding,
    ) -> Result<FrameBinding, Error> {
        self.local_current(window, index)?;
        #[cfg(feature = "profiling")]
        record_owned_storage(Cost::Move(2));
        Ok(self.slots[window.locals().start + usize::from(index)]
            .replace(value)
            .unwrap())
    }

    #[inline]
    fn replace_local_pending_current(
        &mut self,
        window: &FrameWindow,
        index: u16,
        value: &mut Option<FrameBinding>,
    ) -> Result<FrameBinding, Error> {
        self.local_current(window, index)?;
        #[cfg(feature = "profiling")]
        record_owned_storage(Cost::Move(2));
        Ok(self.slots[window.locals().start + usize::from(index)]
            .replace(value.take().expect("pending local owner"))
            .unwrap())
    }

    /// Snapshot the current parameter bindings only across the actual argv
    /// span. Default derived forwarding must not include padded formal slots.
    pub(in crate::engine::vm) fn snapshot_actual_arguments(
        &self,
        window: &FrameWindow,
        runtime: &crate::engine::api::runtime::Runtime,
    ) -> Result<Vec<JsValue>, Error> {
        self.snapshot_argument_tail(window, runtime, 0)
    }

    pub(in crate::engine::vm) fn actual_argument_count(
        &self,
        window: &FrameWindow,
    ) -> Result<usize, Error> {
        self.check_current(window)?;
        Ok(window.actual_count)
    }

    pub(in crate::engine::vm) fn snapshot_argument_tail(
        &self,
        window: &FrameWindow,
        runtime: &Runtime,
        start: usize,
    ) -> Result<Vec<JsValue>, Error> {
        self.check_current(window)?;
        let count = window.actual_count;
        if count > window.parameters().len() || start > window.parameters().len() {
            return Err(Error::internal(
                "actual argument count exceeds parameter window",
            ));
        }
        let start = start.min(count);
        let mut arguments = Vec::new();
        arguments
            .try_reserve_exact(count - start)
            .map_err(|_| Error::internal("argument snapshot allocation failed"))?;
        for index in window.parameters().start + start..window.parameters().start + count {
            let binding = self.slots[index]
                .as_ref()
                .ok_or_else(|| Error::internal("owned parameter is vacant"))?;
            arguments.push(crate::engine::vm::bindings::read_frame_binding(
                runtime, binding,
            )?);
        }
        Ok(arguments)
    }

    pub(in crate::engine::vm) fn parameter(
        &self,
        window: &FrameWindow,
        index: u16,
    ) -> Result<&FrameBinding, Error> {
        self.check_current(window)?;
        self.parameter_current(window, index)
    }

    #[inline]
    fn parameter_current(&self, window: &FrameWindow, index: u16) -> Result<&FrameBinding, Error> {
        if usize::from(index) >= window.parameters().len() {
            return Err(Error::internal("owned parameter index is out of bounds"));
        }
        self.slots[window.parameters().start + usize::from(index)]
            .as_ref()
            .ok_or_else(|| Error::internal("owned parameter is vacant"))
    }

    #[cfg(test)]
    pub(in crate::engine::vm) fn replace_parameter(
        &mut self,
        window: &FrameWindow,
        index: u16,
        value: FrameBinding,
    ) -> Result<FrameBinding, Error> {
        self.check_current(window)?;
        self.replace_parameter_current(window, index, value)
    }

    fn replace_parameter_current(
        &mut self,
        window: &FrameWindow,
        index: u16,
        value: FrameBinding,
    ) -> Result<FrameBinding, Error> {
        self.parameter_current(window, index)?;
        #[cfg(feature = "profiling")]
        record_owned_storage(Cost::Move(2));
        Ok(self.slots[window.parameters().start + usize::from(index)]
            .replace(value)
            .unwrap())
    }

    /// One-way temporary migration handoff. No serialization or extra retain:
    /// the caller receives the very owners which occupied this window.
    pub(in crate::engine::vm) fn take_frame(
        &mut self,
        runtime: &Runtime,
        window: FrameWindow,
    ) -> Result<FrameStorage, Error> {
        self.check_current(&window)?;
        let taken = self.take_frame_owners(runtime, &window)?;
        self.complete_take_frame(window, taken)
    }

    fn take_frame_owners(
        &mut self,
        runtime: &Runtime,
        window: &FrameWindow,
    ) -> Result<FrameStorage, Error> {
        let mut taken = FrameStorage {
            original_arguments: Vec::new(),
            parameters: Vec::new(),
            locals: Vec::new(),
            operands: Vec::new(),
        };
        let result = self.take_frame_span(window, &mut taken);
        if let Err(error) = result {
            // Surrender every owner moved out before the malformed slot.
            for value in taken.original_arguments.drain(..) {
                runtime
                    .release_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
            }
            for binding in taken.parameters.drain(..).chain(taken.locals.drain(..)) {
                release_binding(runtime, binding)?;
            }
            for value in taken.operands.drain(..) {
                runtime
                    .release_jsvalue(value)
                    .map_err(runtime_error_to_vm_error)?;
            }
            return Err(error);
        }
        Ok(taken)
    }

    fn take_frame_span(
        &mut self,
        window: &FrameWindow,
        taken: &mut FrameStorage,
    ) -> Result<(), Error> {
        // Reserve every destination before removing any source owner. A failed
        // migration allocation leaves the complete window owned by this store.
        taken
            .original_arguments
            .try_reserve_exact(window.actual_count)
            .map_err(|_| Error::internal("original argument handoff allocation failed"))?;
        taken
            .parameters
            .try_reserve_exact(window.parameters().len())
            .map_err(|_| Error::internal("parameter handoff allocation failed"))?;
        taken
            .locals
            .try_reserve_exact(window.locals().len())
            .map_err(|_| Error::internal("local handoff allocation failed"))?;
        taken
            .operands
            .try_reserve_exact(window.depth)
            .map_err(|_| Error::internal("operand handoff allocation failed"))?;
        for index in window.original_arguments() {
            let Some(FrameBinding::Direct(value)) = self.slots[index].take() else {
                return Err(Error::internal("original argument is not an owned value"));
            };
            taken.original_arguments.push(value);
        }
        // Only unobservable scalar originals may be absent. Preserve arity
        // for explicit legacy handoff without inventing reference owners.
        #[cfg(feature = "profiling")]
        let omitted = window.actual_count - taken.original_arguments.len();
        taken
            .original_arguments
            .resize(window.actual_count, JsValue::Undefined);
        for index in window.parameters() {
            taken.parameters.push(self.slots[index].take().unwrap());
        }
        for index in window.locals() {
            taken.locals.push(self.slots[index].take().unwrap());
        }
        for index in window.operands().start..window.operands().start + window.depth {
            let Some(FrameBinding::Direct(value)) = self.slots[index].take() else {
                return Err(Error::internal("operand is not an owned value"));
            };
            taken.operands.push(value);
        }
        #[cfg(feature = "profiling")]
        {
            self.omitted = omitted;
        }
        Ok(())
    }

    fn complete_take_frame(
        &mut self,
        window: FrameWindow,
        taken: FrameStorage,
    ) -> Result<FrameStorage, Error> {
        #[cfg(feature = "profiling")]
        {
            let moved =
                taken.original_arguments.len() + taken.parameters.len() + taken.locals.len()
                    + taken.operands.len();
            self.live_slots -= moved - self.omitted;
            record_owned_storage(Cost::Move(moved));
        }
        debug_assert!(self.slots[window.whole()].iter().all(Option::is_none));
        self.active_end = window.whole().start;
        self.windows.pop();
        Ok(taken)
    }

    /// The driver must keep the completion/pending result owned before
    /// clearing. Every released binding's edges are surrendered through the
    /// runtime's deferred-release path.
    pub(in crate::engine::vm) fn clear_frame(
        &mut self,
        runtime: &Runtime,
        window: FrameWindow,
    ) -> Result<(), Error> {
        self.check_current(&window)?;
        #[cfg(feature = "profiling")]
        {
            let cleared = self.slots[window.whole()]
                .iter()
                .filter(|slot| slot.is_some())
                .count();
            self.live_slots -= cleared;
            record_owned_storage(Cost::Clear(cleared));
        }
        // Vec::truncate previously lowered logical length before dropping the
        // suffix. Preserve that authority boundary and ascending owner order.
        self.active_end = window.whole().start;
        for index in window.whole().start..window.operands().start + window.depth {
            if let Some(binding) = self.slots[index].take() {
                release_binding(runtime, binding)?;
            }
        }
        debug_assert!(self.slots[window.whole()].iter().all(Option::is_none));
        self.windows.pop();
        Ok(())
    }
}

/// The running stack's copy boundary. Scalars copy inline; every heap-backed
/// kind duplicates its edge through the runtime. Releases are separate, so a
/// failed retain cannot repeat a committed release.
#[inline(always)]
pub(in crate::engine::vm) fn copy_value(
    runtime: &Runtime,
    value: &JsValue,
) -> Result<JsValue, Error> {
    let copied = match value {
        JsValue::Undefined => JsValue::Undefined,
        JsValue::Null => JsValue::Null,
        JsValue::Bool(value) => JsValue::Bool(*value),
        JsValue::Int(value) => JsValue::Int(*value),
        JsValue::Float(value) => JsValue::Float(*value),
        _ => return copy_reference(runtime, value),
    };
    #[cfg(feature = "profiling")]
    record_copy(value);
    Ok(copied)
}

// Keep fallible heap retains and their error formatting out of scalar copies.
// This is not cold: String/BigInt copies also share this boundary.
#[inline(never)]
fn copy_reference(runtime: &Runtime, value: &JsValue) -> Result<JsValue, Error> {
    let copied = runtime
        .dup_jsvalue(value)
        .map_err(|error| Error::internal(error.to_string()))?;
    #[cfg(feature = "profiling")]
    record_copy(value);
    Ok(copied)
}

#[cfg(feature = "profiling")]
fn record_copy(value: &JsValue) {
    record_owned_storage(Cost::Copy {
        heap_root: matches!(value, JsValue::Object(_) | JsValue::Symbol(_)),
    });
    crate::engine::api::profiling::record_owned_execution_event(match value {
        JsValue::String(_) => "slot_copy.StringNode",
        JsValue::BigInt(_) => "slot_copy.BigIntNode",
        JsValue::Object(_) => "slot_copy.ObjectRetain",
        JsValue::Symbol(_) => "slot_copy.SymbolRetain",
        _ => "slot_copy.Immediate",
    });
}

#[cfg(feature = "profiling")]
impl Drop for SlotStore {
    fn drop(&mut self) {
        record_owned_storage(Cost::Clear(self.live_slots));
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn owned_property_ic_capacity_preflight_and_receiver_forms_preserve_owners() {
        use crate::engine::code::bytecode::Instruction;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("(function(o){return o.x})").unwrap())
            .unwrap();
        let crate::engine::vm::call::CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("bytecode")
        };
        let code = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        let (pc, key) = code
            .code
            .iter()
            .enumerate()
            .find_map(|(pc, op)| match op {
                Instruction::GetField(key) => Some((pc, *key)),
                _ => None,
            })
            .unwrap();
        let base = context
            .eval("globalThis.icSlotValue={marker:1};({x:icSlotValue})")
            .unwrap();
        let value = context.eval("icSlotValue").unwrap();
        let Value::Object(value_object) = &value else {
            panic!("object")
        };
        let mut native = None;
        assert!(
            runtime
                .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
                .unwrap()
                .is_none()
        );
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 1;
        let mut slots = SlotStore::new(2);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        slots.push(&mut window, base.clone()).unwrap();
        let count = runtime
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(value_object.object_id())
            .unwrap();
        assert!(
            !slots
                .run_window(&mut window)
                .unwrap()
                .property_ic_read(&runtime, &code, pc, key, true, &mut native)
                .unwrap()
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(value_object.object_id())
                .unwrap(),
            count
        );
        assert_eq!(window.depth, 1);
        assert_eq!(slots.peek(&window, 0).unwrap(), &base);
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .property_ic_read(&runtime, &code, pc, key, false, &mut native)
                .unwrap()
        );
        assert_eq!(window.depth, 1);
        assert_eq!(slots.peek(&window, 0).unwrap(), &value);
        slots.clear_frame(window).unwrap();
        owner.metadata.max_stack = 2;
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        slots.push(&mut window, base.clone()).unwrap();
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .property_ic_read(&runtime, &code, pc, key, true, &mut native)
                .unwrap()
        );
        assert_eq!(window.depth, 2);
        assert_eq!(slots.peek(&window, 0).unwrap(), &value);
        assert_eq!(slots.peek(&window, 1).unwrap(), &base);
        slots.clear_frame(window).unwrap();
    }

    use super::{FrameStorage, SlotStore};
    use crate::engine::api::Runtime;
    use crate::engine::code::runtime::PublishedFunctionSnapshot;
    use crate::engine::value::Value;
    use crate::engine::vm::bindings::{FrameBinding, release_frame_binding as release_binding};

    #[test]
    fn native_argument_transaction_preserves_order_and_surviving_owners() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 6;
        let mut slots = SlotStore::new(6);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let receiver = context.eval("({tag:1})").unwrap();
        let argument = context.eval("({tag:2})").unwrap();
        let callable = context.eval("Math.min").unwrap();
        for value in [
            Value::Int(99),
            receiver.clone(),
            callable,
            Value::Int(1),
            argument.clone(),
            Value::Int(3),
        ] {
            slots.push(&mut window, value).unwrap();
        }
        assert!(
            slots
                .validate_call_value_domains(&window, &runtime, 3, true)
                .unwrap()
        );
        let (arguments, moved_receiver) = slots
            .take_native_call_operands(&mut window, 3, true)
            .unwrap();
        assert_eq!(arguments, [Value::Int(1), argument, Value::Int(3)]);
        assert_eq!(moved_receiver, receiver);
        assert_eq!(slots.depth(&window), 1);
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(99));
        drop(arguments);
        drop(moved_receiver);
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn ordinary_field_leaf_declines_without_consuming_stack_inputs() {
        use crate::engine::code::bytecode::Instruction;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("(function(o,v){o.x=v;return o.x})").unwrap())
            .unwrap();
        let crate::engine::vm::call::CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("bytecode")
        };
        let code = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        let key = code
            .code
            .iter()
            .find_map(|op| match op {
                Instruction::GetField(index) => Some(*index),
                _ => None,
            })
            .unwrap();
        let pc = code
            .code
            .iter()
            .position(|op| matches!(op, Instruction::PutField(_)))
            .unwrap();
        for source in [
            "({get x(){throw 42}})",
            "({x:'reference'})",
            "Object.create({x:42})",
        ] {
            let base = context.eval(source).unwrap();
            let _retained = base.clone();
            let Value::Object(root) = &base else {
                unreachable!()
            };
            let id = root.object_id();
            let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
            owner.metadata.max_stack = 3;
            let mut slots = SlotStore::new(3);
            let mut window = slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .unwrap();
            slots.push(&mut window, Value::Int(99)).unwrap();
            slots.push(&mut window, base).unwrap();
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .ordinary_field_immediate_read(&runtime, &code, key)
                    .unwrap()
            );
            assert_eq!(window.depth, 2);
            assert!(
                matches!(slots.peek(&window,0).unwrap(),Value::Object(root) if root.object_id()==id)
            );
            slots.push(&mut window, Value::Int(17)).unwrap();
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .property_ic_write_scalar(&runtime, &code, pc, key)
                    .unwrap()
            );
            assert_eq!(window.depth, 3);
            assert_eq!(slots.peek(&window, 0).unwrap(), &Value::Int(17));
            assert!(
                matches!(slots.peek(&window,1).unwrap(),Value::Object(root) if root.object_id()==id)
            );
            assert_eq!(slots.peek(&window, 2).unwrap(), &Value::Int(99));
            slots.clear_frame(window).unwrap();
        }
    }

    #[test]
    fn call_domain_validation_preserves_receiver_then_argument_rejection_order() {
        for (foreign_receiver, foreign_first, malformed_first) in [
            (true, false, true),
            (false, true, false),
            (false, false, true),
        ] {
            let runtime = Runtime::new();
            let foreign = Runtime::new();
            let context = runtime.new_context();
            let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
            owner.metadata.max_stack = 4;
            let mut slots = SlotStore::new(4);
            let mut window = slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .unwrap();
            let receiver = Value::Object(if foreign_receiver {
                foreign.new_object(None).unwrap()
            } else {
                runtime.new_object(None).unwrap()
            });
            let first = Value::Object(if foreign_first {
                foreign.new_object(None).unwrap()
            } else {
                runtime.new_object(None).unwrap()
            });
            // Caller shape is [receiver, callee, first argument, second argument].
            for value in [
                receiver,
                Value::Int(0),
                first,
                Value::Object(foreign.new_object(None).unwrap()),
            ] {
                slots.push(&mut window, value).unwrap();
            }
            let removed_index = if malformed_first { 2 } else { 3 };
            let removed = slots.slots[removed_index].take();
            let result = slots.validate_call_value_domains(&window, &runtime, 2, true);
            if foreign_receiver || foreign_first {
                assert!(result.is_err());
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("owned operand slot is not a value")
                );
            }
            assert_eq!(window.depth, 4);
            assert!(slots.slots[removed_index].is_none());
            slots.slots[removed_index] = removed;
            slots.clear_frame(window).unwrap();
        }
    }

    #[test]
    fn call_domain_validation_borrows_values_without_heap_borrow_or_owner_changes() {
        let runtime = Runtime::new();
        let foreign = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 3;
        let mut slots = SlotStore::new(3);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let local = runtime.new_object(None).unwrap();
        let id = local.object_id();
        for value in [Value::Int(0), Value::Object(local), Value::Int(42)] {
            slots.push(&mut window, value).unwrap();
        }
        {
            let state = runtime.0.state.borrow_mut();
            assert!(
                slots
                    .validate_call_value_domains(&window, &runtime, 2, false)
                    .unwrap()
            );
            assert!(
                slots
                    .validate_call_value_domains(&window, &foreign, 2, false)
                    .is_err()
            );
            assert!(state.heap.object(id).is_ok());
        }
        assert_eq!(window.depth, 3);
        assert_eq!(slots.peek(&window, 0).unwrap(), &Value::Int(42));
        assert!(
            matches!(slots.peek(&window, 1).unwrap(), Value::Object(root) if root.object_id()==id)
        );
        slots.clear_frame(window).unwrap();
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
    }

    #[test]
    fn call_domain_validation_rejects_foreign_and_stale_windows_without_consuming_inputs() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 2;
        let mut slots = SlotStore::new(4);
        let mut parent = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        slots.push(&mut parent, Value::Int(0)).unwrap();
        slots.push(&mut parent, Value::Int(42)).unwrap();
        let other = SlotStore::new(2);
        assert!(
            other
                .validate_call_value_domains(&parent, &runtime, 1, false)
                .is_err()
        );
        let child = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        assert!(
            slots
                .validate_call_value_domains(&parent, &runtime, 1, false)
                .is_err()
        );
        assert_eq!(parent.depth, 2);
        slots.clear_frame(child).unwrap();
        assert!(
            slots
                .validate_call_value_domains(&parent, &runtime, 1, false)
                .unwrap()
        );
        assert_eq!(slots.pop(&mut parent).unwrap(), Value::Int(42));
        assert_eq!(slots.pop(&mut parent).unwrap(), Value::Int(0));
        slots.clear_frame(parent).unwrap();
    }

    #[test]
    fn typed_number_leaf_declines_without_consuming_or_writing_inputs() {
        for (source, key, object_value, single_root, detached) in [
            ("new Uint8Array(1)", 1, false, false, false),
            ("new Uint8Array(1)", -1, false, false, false),
            ("new Uint8Array(1)", 0, true, false, false),
            ("new Uint8Array(1)", 0, false, true, false),
            ("new Uint8Array(1)", 0, false, false, true),
            (
                "new Uint8Array(new SharedArrayBuffer(1))",
                0,
                false,
                false,
                false,
            ),
            ("new BigInt64Array(1)", 0, false, false, false),
            ("({})", 0, false, false, false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let base = context.eval(source).unwrap();
            let retained = (!single_root).then(|| base.clone());
            if single_root {
                assert_ne!(
                    runtime.slot_value_release_readiness(&base).unwrap(),
                    crate::engine::heap::SlotReleaseReadiness::Ready
                );
            }
            if detached {
                // Use the actual backing of this view, without adding a view owner.
                let Value::Object(view) = &base else {
                    unreachable!()
                };
                let snapshot = runtime.typed_array_snapshot(view).unwrap();
                let backing = crate::engine::object::ObjectRef::from_borrowed_handle(
                    runtime.clone(),
                    snapshot.buffer,
                )
                .unwrap();
                context
                    .detach_array_buffer(&Value::Object(backing))
                    .unwrap();
            }
            let value = if object_value {
                context.eval("({valueOf(){throw 42}})").unwrap()
            } else {
                Value::Int(17)
            };
            let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
            owner.metadata.max_stack = 3;
            let mut slots = SlotStore::new(3);
            let mut window = slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .unwrap();
            for value in [base, Value::Int(key), value] {
                slots.push(&mut window, value).unwrap();
            }
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .typed_array_number_write(&runtime)
                    .unwrap(),
                "{source}"
            );
            assert_eq!(window.depth, 3);
            assert_eq!(slots.peek(&window, 1).unwrap(), &Value::Int(key));
            if !object_value {
                assert_eq!(slots.peek(&window, 0).unwrap(), &Value::Int(17));
            }
            if let Some(Value::Object(view)) = &retained {
                if source.starts_with("new Uint8Array") && !detached {
                    assert_eq!(
                        runtime.typed_array_read_index(view, 0).unwrap(),
                        Some(Value::Int(0))
                    );
                }
            }
            slots.clear_frame(window).unwrap();
        }
    }

    #[test]
    fn typed_number_leaf_preserves_deferred_and_borrowed_inputs_until_fallback() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let base = context.eval("new Uint8Array(1)").unwrap();
        let retained = base.clone();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 3;
        let mut slots = SlotStore::new(3);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        for value in [base, Value::Int(0), Value::Int(17)] {
            slots.push(&mut window, value).unwrap();
        }
        {
            let _borrow = runtime.0.state.borrow();
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .typed_array_number_write(&runtime)
                    .unwrap()
            );
            assert_eq!(window.depth, 3);
        }
        let queued = runtime.new_object(None).unwrap();
        {
            let _borrow = runtime.0.state.borrow();
            drop(queued);
        }
        assert!(
            !slots
                .run_window(&mut window)
                .unwrap()
                .typed_array_number_write(&runtime)
                .unwrap()
        );
        assert_eq!(window.depth, 3);
        assert_eq!(slots.peek(&window, 0).unwrap(), &Value::Int(17));
        assert!(runtime.0.deferred_references.has_pending());
        runtime.drain_deferred_references().unwrap();
        let Value::Object(view) = &retained else {
            unreachable!()
        };
        assert_eq!(
            runtime.typed_array_read_index(view, 0).unwrap(),
            Some(Value::Int(0))
        );
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .typed_array_number_write(&runtime)
                .unwrap()
        );
        assert_eq!(window.depth, 0);
        assert_eq!(
            runtime.typed_array_read_index(view, 0).unwrap(),
            Some(Value::Int(17))
        );
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn recovery_string_index_leaf_preserves_spelling_and_final_key_owner() {
        for (text, retained, expected) in [
            ("0", true, true),
            ("0", false, false),
            ("01", true, false),
            ("-0", true, false),
            ("4294967295", true, false),
            ("1e0", true, false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let base = context.eval("[42]").unwrap();
            let keep_base = base.clone();
            let key = crate::engine::value::JsString::try_from_utf8(text).unwrap();
            let keep_key = retained.then(|| key.clone());
            let mut code = PublishedFunctionSnapshot::empty_for_test(context.realm);
            code.metadata.max_stack = 2;
            let mut store = SlotStore::new(2);
            let mut window = store
                .push_frame(&code.frame_layout(), empty_storage())
                .unwrap();
            store.push(&mut window, base).unwrap();
            store.push(&mut window, Value::String(key)).unwrap();
            assert_eq!(
                store
                    .run_window(&mut window)
                    .unwrap()
                    .array_immediate_read(&runtime)
                    .unwrap(),
                expected,
                "{text}/{retained}"
            );
            if expected {
                assert_eq!(store.peek(&window, 0).unwrap(), &Value::Int(42));
                assert_eq!(window.depth, 1);
            } else {
                assert_eq!(window.depth, 2);
                assert!(matches!(store.peek(&window, 0).unwrap(), Value::String(_)));
            }
            store.clear_frame(window).unwrap();
            drop(keep_key);
            drop(keep_base);
        }
    }

    #[test]
    fn dense_read_leaf_preserves_declined_inputs_and_neighboring_operands() {
        for (source, key, single_root) in [
            ("[42]", Value::Int(0), true),
            ("[42]", Value::Int(1), false),
            ("[42]", Value::Int(-1), false),
            ("[42]", Value::Float(0.0), false),
            ("[{}]", Value::Int(0), false),
            ("['text']", Value::Int(0), false),
            ("Object.create({0:42})", Value::Int(0), false),
            ("new Proxy([42],{get(){throw 42}})", Value::Int(0), false),
            (
                "Object.defineProperty([],0,{get(){throw 42}})",
                Value::Int(0),
                false,
            ),
            ("[,]", Value::Int(0), false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let base = context.eval(source).unwrap();
            let _retained = (!single_root).then(|| base.clone());
            let Value::Object(root) = &base else {
                unreachable!()
            };
            let id = root.object_id();
            let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
            owner.metadata.max_stack = 3;
            let mut slots = SlotStore::new(3);
            let mut window = slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .unwrap();
            for value in [Value::Int(99), base, key.clone()] {
                slots.push(&mut window, value).unwrap();
            }
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .array_immediate_read(&runtime)
                    .unwrap(),
                "{source}"
            );
            assert_eq!(window.depth, 3);
            assert_eq!(slots.peek(&window, 0).unwrap(), &key);
            assert!(
                matches!(slots.peek(&window, 1).unwrap(), Value::Object(root) if root.object_id()==id)
            );
            assert_eq!(slots.peek(&window, 2).unwrap(), &Value::Int(99));
            slots.clear_frame(window).unwrap();
        }
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let base = context.eval("[42]").unwrap();
        let retained = base.clone();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 3;
        let mut slots = SlotStore::new(3);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        for value in [Value::Int(99), base, Value::Int(0)] {
            slots.push(&mut window, value).unwrap();
        }
        let queued = runtime.new_object(None).unwrap();
        {
            let _borrow = runtime.0.state.borrow();
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .array_immediate_read(&runtime)
                    .unwrap()
            );
            drop(queued);
        }
        assert!(
            !slots
                .run_window(&mut window)
                .unwrap()
                .array_immediate_read(&runtime)
                .unwrap()
        );
        assert_eq!(window.depth, 3);
        assert!(runtime.0.deferred_references.has_pending());
        runtime.drain_deferred_references().unwrap();
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .array_immediate_read(&runtime)
                .unwrap()
        );
        assert_eq!(window.depth, 2);
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(42));
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(99));
        drop(retained);
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn typed_read_leaf_preserves_declined_inputs_and_release_guards() {
        for (source, single_root) in [
            ("new Uint8Array([42])", true),
            ("new Uint8Array(0)", false),
            (
                "(function(){var a=new Uint8Array(1);a.buffer.transfer();return a})()",
                false,
            ),
            (
                "(function(){var b=new ArrayBuffer(4,{maxByteLength:8}),a=new Uint8Array(b,2,2);b.resize(1);return a})()",
                false,
            ),
            ("new Uint8Array(new SharedArrayBuffer(1))", false),
            ("new BigInt64Array([42n])", false),
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let base = context.eval(source).unwrap();
            let _retained = (!single_root).then(|| base.clone());
            let Value::Object(root) = &base else {
                unreachable!()
            };
            let id = root.object_id();
            let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
            owner.metadata.max_stack = 2;
            let mut slots = SlotStore::new(2);
            let mut window = slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .unwrap();
            for value in [base, Value::Int(0)] {
                slots.push(&mut window, value).unwrap();
            }
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .array_immediate_read(&runtime)
                    .unwrap(),
                "{source}"
            );
            assert_eq!(window.depth, 2);
            assert_eq!(slots.peek(&window, 0).unwrap(), &Value::Int(0));
            assert!(
                matches!(slots.peek(&window, 1).unwrap(), Value::Object(root) if root.object_id()==id)
            );
            slots.clear_frame(window).unwrap();
        }
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let base = context.eval("new Int32Array([42])").unwrap();
        let _retained = base.clone();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 2;
        let mut slots = SlotStore::new(2);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        for value in [base, Value::Int(0)] {
            slots.push(&mut window, value).unwrap();
        }
        let queued = runtime.new_object(None).unwrap();
        {
            let _borrow = runtime.0.state.borrow();
            assert!(
                !slots
                    .run_window(&mut window)
                    .unwrap()
                    .array_immediate_read(&runtime)
                    .unwrap()
            );
            drop(queued);
        }
        assert!(
            !slots
                .run_window(&mut window)
                .unwrap()
                .array_immediate_read(&runtime)
                .unwrap()
        );
        assert_eq!(window.depth, 2);
        assert!(runtime.0.deferred_references.has_pending());
        runtime.drain_deferred_references().unwrap();
        assert!(
            slots
                .run_window(&mut window)
                .unwrap()
                .array_immediate_read(&runtime)
                .unwrap()
        );
        assert_eq!(window.depth, 1);
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(42));
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn compact_window_boundaries_keep_adjacent_regions_and_parent_authority() {
        use crate::engine::code::function::metadata::{ClosureVariableKind, VariableDefinition};
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut caller = PublishedFunctionSnapshot::empty_for_test(context.realm);
        caller.metadata.max_stack = 1;
        let mut callee = PublishedFunctionSnapshot::empty_for_test(context.realm);
        callee.metadata.argument_count = 2;
        callee.metadata.local_count = 2;
        callee.metadata.max_stack = 2;
        callee.local_definitions = std::rc::Rc::from(
            [VariableDefinition {
                name: None,
                is_lexical: false,
                is_const: false,
                is_parameter_initializer: false,
                kind: ClosureVariableKind::Normal,
            }; 2],
        );
        let mut slots = SlotStore::new(16);
        let mut parent = slots
            .push_frame(&caller.frame_layout(), empty_storage())
            .unwrap();
        slots.push(&mut parent, Value::Int(99)).unwrap();
        let mut child = slots
            .push_frame(
                &callee.frame_layout(),
                FrameStorage {
                    original_arguments: vec![Value::Int(10)],
                    parameters: vec![
                        FrameBinding::Direct(Value::Int(10)),
                        FrameBinding::Direct(Value::Undefined),
                    ],
                    locals: vec![
                        FrameBinding::Direct(Value::Int(20)),
                        FrameBinding::Uninitialized,
                    ],
                    operands: vec![Value::Int(30)],
                },
            )
            .unwrap();
        assert_eq!(child.whole(), 1..8);
        assert_eq!(child.original_arguments(), 1..2);
        assert_eq!(child.parameters(), 2..4);
        assert_eq!(child.locals(), 4..6);
        assert_eq!(child.operands(), 6..8);
        assert!(slots.peek(&parent, 0).is_err());
        assert!(slots.parameter(&child, 2).is_err());
        assert!(slots.local(&child, 2).is_err());
        slots.push(&mut child, Value::Int(31)).unwrap();
        assert!(slots.push(&mut child, Value::Int(32)).is_err());
        let end = child.end;
        child.end = end - 1;
        assert!(slots.pop(&mut child).is_err());
        child.end = end;
        assert_eq!(slots.depth(&child), 2);
        assert_eq!(slots.pop(&mut child).unwrap(), Value::Int(31));
        let storage = slots.take_frame(child).unwrap();
        assert_eq!(storage.original_arguments, vec![Value::Int(10)]);
        assert_eq!(storage.operands, vec![Value::Int(30)]);
        assert!(matches!(
            storage.locals[0],
            FrameBinding::Direct(Value::Int(20))
        ));
        assert!(matches!(storage.locals[1], FrameBinding::Uninitialized));
        assert_eq!(slots.pop(&mut parent).unwrap(), Value::Int(99));
        slots.clear_frame(parent).unwrap();
        assert_eq!(slots.active_end, 0);
        assert!(slots.slots.iter().all(Option::is_none));
        #[cfg(target_pointer_width = "64")]
        // Actual arity remains independent when unobservable originals are omitted.
        assert_eq!(std::mem::size_of::<super::FrameWindow>(), 72);
    }

    #[test]
    fn compact_empty_windows_share_boundaries_but_not_identity() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        let mut slots = SlotStore::new(0);
        let parent = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let child = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        for window in [&parent, &child] {
            assert_eq!(window.whole(), 0..0);
            assert_eq!(window.original_arguments(), 0..0);
            assert_eq!(window.parameters(), 0..0);
            assert_eq!(window.locals(), 0..0);
            assert_eq!(window.operands(), 0..0);
        }
        assert_ne!(parent.id, child.id);
        assert!(slots.binding_counts(&parent).is_err());
        assert_eq!(slots.binding_counts(&child).unwrap(), (0, 0));
        slots.clear_frame(child).unwrap();
        assert_eq!(slots.binding_counts(&parent).unwrap(), (0, 0));
        slots.clear_frame(parent).unwrap();
        assert!(slots.windows.is_empty());
    }

    #[test]
    fn outgoing_tail_transfer_is_atomic_and_restores_the_caller_prefix() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut caller = PublishedFunctionSnapshot::empty_for_test(context.realm);
        caller.metadata.max_stack = 5;
        let mut callee = PublishedFunctionSnapshot::empty_for_test(context.realm);
        callee.metadata.argument_count = 3;
        let function = runtime.new_object(None).unwrap();
        let marker = runtime.new_object(None).unwrap();
        let mut slots = SlotStore::new(32);
        let mut parent = slots
            .push_frame(&caller.frame_layout(), empty_storage())
            .unwrap();
        for value in [
            Value::Int(99),
            Value::Object(marker.clone()),
            Value::Object(function.clone()),
            Value::Int(7),
            Value::Object(marker.clone()),
        ] {
            slots.push(&mut parent, value).unwrap();
        }
        {
            // Retaining a root while the state is mutably borrowed is the
            // injected failure: the shared-borrow fast path added for nested
            // materialization only applies to shared borrows.
            let _borrow = runtime.0.state.borrow_mut();
            assert!(
                slots
                    .push_call_frame(
                        &callee.frame_layout(),
                        &mut parent,
                        2,
                        true,
                        &function,
                        None
                    )
                    .is_err()
            );
            assert_eq!(slots.depth(&parent), 5);
            assert_eq!(slots.peek(&parent, 1).unwrap(), &Value::Int(7));
            assert_eq!(slots.active_end, parent.whole().end);
            assert!(slots.slots[slots.active_end..].iter().all(Option::is_none));
            assert_eq!(slots.windows.len(), 1);
        }
        let child = slots
            .push_call_frame(
                &callee.frame_layout(),
                &mut parent,
                2,
                true,
                &function,
                None,
            )
            .unwrap();
        assert_eq!(slots.depth(&parent), 1);
        assert!(slots.peek(&parent, 0).is_err());
        let storage = slots.take_frame(child).unwrap();
        assert_eq!(storage.original_arguments.len(), 2);
        assert_eq!(storage.original_arguments[0], Value::Int(7));
        assert_eq!(storage.parameters.len(), 3);
        assert!(matches!(
            storage.parameters[2],
            FrameBinding::Direct(Value::Undefined)
        ));
        assert_eq!(slots.peek(&parent, 0).unwrap(), &Value::Int(99));
        assert!(
            slots.slots[parent.operands().start + 1..parent.operands().end]
                .iter()
                .all(Option::is_none)
        );
        slots.clear_frame(parent).unwrap();
    }

    #[test]
    fn direct_initialization_preserves_snapshot_padding_and_transactional_failure() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.argument_count = 3;
        let function = runtime.new_object(None).unwrap();
        let object = runtime.new_object(None).unwrap();
        let mut slots = SlotStore::new(16);
        let source = || FrameStorage {
            original_arguments: vec![Value::Int(7), Value::Object(object.clone())],
            parameters: Vec::new(),
            locals: Vec::new(),
            operands: Vec::new(),
        };
        let storage = source();
        {
            // Retaining a root while the state is mutably borrowed is the
            // injected failure: the shared-borrow fast path added for nested
            // materialization only applies to shared borrows.
            let _borrow = runtime.0.state.borrow_mut();
            assert!(
                slots
                    .push_initialized_frame(&owner.frame_layout(), storage, &function, None)
                    .is_err()
            );
            assert_eq!(slots.active_end, 0);
            assert!(slots.slots.iter().all(Option::is_none));
            assert!(slots.windows.is_empty());
        }
        let window = slots
            .push_initialized_frame(&owner.frame_layout(), source(), &function, None)
            .unwrap();
        assert_eq!(slots.binding_counts(&window).unwrap(), (0, 3));
        assert!(matches!(
            slots.parameter(&window, 2).unwrap(),
            FrameBinding::Direct(Value::Undefined)
        ));
        slots
            .replace_parameter(&window, 0, FrameBinding::Direct(Value::Int(9)))
            .unwrap();
        let storage = slots.take_frame(window).unwrap();
        assert_eq!(storage.original_arguments.len(), 2);
        assert_eq!(storage.original_arguments[0], Value::Int(7));
        assert!(matches!(
            storage.parameters[0],
            FrameBinding::Direct(Value::Int(9))
        ));
    }

    #[test]
    fn initialized_high_water_reuses_mixed_depth_three_windows_without_roots() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let marker = runtime.new_object(None).unwrap();
        let marker_id = marker.object_id();
        let mut slots = SlotStore::new(16);
        #[cfg(feature = "profiling")]
        let profile = crate::engine::api::profiling::CostProfile::start();
        let mut capacity = None;
        for _ in 0..20 {
            let mut windows = Vec::new();
            for size in [2, 7, 3] {
                let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
                owner.metadata.max_stack = size;
                let mut window = slots
                    .push_frame(&owner.frame_layout(), empty_storage())
                    .unwrap();
                slots
                    .push(&mut window, Value::Object(marker.clone()))
                    .unwrap();
                windows.push(window);
            }
            assert_eq!(slots.active_end, 12);
            assert_eq!(slots.slots.len(), 12);
            while let Some(window) = windows.pop() {
                slots.clear_frame(window).unwrap();
                assert!(slots.slots[slots.active_end..].iter().all(Option::is_none));
                if let Some(parent) = windows.last() {
                    assert_eq!(
                        slots.peek(parent, 0).unwrap(),
                        &Value::Object(marker.clone())
                    );
                }
            }
            assert_eq!(slots.active_end, 0);
            assert_eq!(slots.slots.len(), 12);
            if let Some(previous) = capacity {
                assert_eq!(slots.slots.capacity(), previous);
            }
            capacity = Some(slots.slots.capacity());
        }
        #[cfg(feature = "profiling")]
        {
            let costs = profile.snapshot().owned_storage;
            assert_eq!(costs.physical_none_initializations, 12);
            assert_eq!(costs.maximum_initialized_slots, 12);
            assert_eq!(costs.maximum_reserved_slots, 12);
            assert_eq!(costs.slots_initialized, 240);
            assert_eq!(costs.maximum_live_slots, 3);
        }
        drop(marker);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(marker_id).is_err());
        assert!(slots.slots.iter().all(Option::is_none));
    }

    #[test]
    fn initialized_suffix_rolls_back_after_a_successful_object_copy() {
        let runtime = Runtime::new();
        let other_runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.argument_count = 3;
        let function = runtime.new_object(None).unwrap();
        let first = runtime.new_object(None).unwrap();
        let first_id = first.object_id();
        let blocked = other_runtime.new_object(None).unwrap();
        let storage = FrameStorage {
            original_arguments: vec![Value::Object(first.clone()), Value::Object(blocked.clone())],
            ..empty_storage()
        };
        let mut slots = SlotStore::new(16);
        {
            // First retain succeeds in its runtime; the second retain fails.
            // A mutable borrow injects the failure (shared borrows take the
            // nested-materialization fast path and retain successfully).
            let _borrow = other_runtime.0.state.borrow_mut();
            assert!(
                slots
                    .push_initialized_frame(&owner.frame_layout(), storage, &function, None)
                    .is_err()
            );
            assert_eq!(slots.active_end, 0);
            assert!(slots.windows.is_empty());
            assert!(slots.slots.iter().all(Option::is_none));
        }
        drop(first);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(first_id).is_err());
        let window = slots
            .push_initialized_frame(&owner.frame_layout(), empty_storage(), &function, None)
            .unwrap();
        slots.clear_frame(window).unwrap();
        assert!(slots.slots.iter().all(Option::is_none));
        other_runtime.run_gc().unwrap();
    }

    #[test]
    fn initialized_backing_preserves_take_frame_suspension_handoff_ownership() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.argument_count = 2;
        owner.metadata.max_stack = 5;
        let value = runtime.new_object(None).unwrap();
        let value_id = value.object_id();
        let mut slots = SlotStore::new(16);
        let window = slots
            .push_frame(
                &owner.frame_layout(),
                FrameStorage {
                    original_arguments: vec![Value::Int(1), Value::Int(2)],
                    parameters: vec![
                        FrameBinding::Direct(Value::Int(3)),
                        FrameBinding::Direct(Value::Int(4)),
                    ],
                    locals: Vec::new(),
                    operands: vec![Value::Object(value)],
                },
            )
            .unwrap();
        let storage = slots.take_frame(window).unwrap();
        let initialized = slots.slots.len();
        assert_eq!(slots.active_end, 0);
        assert!(slots.slots.iter().all(Option::is_none));
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(value_id).is_ok());
        let window = slots.push_frame(&owner.frame_layout(), storage).unwrap();
        assert_eq!(slots.slots.len(), initialized);
        assert!(
            matches!(slots.peek(&window, 0).unwrap(), Value::Object(value) if value.object_id() == value_id)
        );
        slots.clear_frame(window).unwrap();
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(value_id).is_err());
        assert!(slots.slots.iter().all(Option::is_none));
    }

    fn empty_storage() -> FrameStorage {
        FrameStorage {
            original_arguments: Vec::new(),
            parameters: Vec::new(),
            locals: Vec::new(),
            operands: Vec::new(),
        }
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn copy_cost_distinguishes_short_bigints_from_shared_heap_bigints() {
        let short = Value::BigInt("1".parse().unwrap());
        let heap = Value::BigInt("18446744073709551616".parse().unwrap());
        let profile = crate::engine::api::profiling::CostProfile::start();
        assert_eq!(super::copy_value(&short).unwrap(), short);
        assert_eq!(super::copy_value(&heap).unwrap(), heap);
        let cost = profile.snapshot();
        assert_eq!(cost.owned_storage.value_copies, 2);
        assert_eq!(cost.owned_storage.copied_heap_roots, 0);
        assert_eq!(cost.owned_execution_events["slot_copy.BigIntImmediate"], 1);
        assert_eq!(cost.owned_execution_events["slot_copy.BigIntRc"], 1);
    }

    #[test]
    fn numeric_replacement_is_transactional_and_clears_the_dead_owner() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 2;
        let mut slots = SlotStore::new(8);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        assert!(
            slots
                .binary_number(&mut window, |_, _| unreachable!())
                .is_err()
        );
        slots.push(&mut window, Value::Int(7)).unwrap();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        slots.push(&mut window, Value::Object(object)).unwrap();
        assert!(
            !slots
                .binary_number(&mut window, |_, _| unreachable!())
                .unwrap()
        );
        assert_eq!(slots.depth(&window), 2);
        assert_eq!(slots.peek(&window, 1).unwrap(), &Value::Int(7));
        assert!(
            matches!(slots.peek(&window, 0).unwrap(), Value::Object(root) if root.object_id() == id)
        );
        drop(slots.pop(&mut window).unwrap());
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        slots.push(&mut window, Value::Int(3)).unwrap();
        let child = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        assert!(
            slots
                .binary_number(&mut window, |_, _| unreachable!())
                .is_err()
        );
        slots.clear_frame(child).unwrap();
        assert!(
            slots
                .binary_number(&mut window, |left, right| left.sub(right).into())
                .unwrap()
        );
        assert_eq!(slots.depth(&window), 1);
        assert!(slots.slots[window.operands().start + 1].is_none());
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(4));
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn parent_window_survives_growth_and_last_result_outlives_clear() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 1;
        let mut slots = SlotStore::new(8192);
        let mut parent = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        slots.push(&mut parent, Value::Object(object)).unwrap();
        let original_capacity = slots.slots.capacity();
        owner.metadata.max_stack = 4096;
        let child = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        assert!(slots.slots.capacity() > original_capacity);
        assert!(
            slots.peek(&parent, 0).is_err(),
            "an inactive parent cannot access the current window"
        );
        slots.clear_frame(child).unwrap();
        let result = slots.pop(&mut parent).unwrap();
        assert!(slots.slots[parent.operands().start].is_none());
        slots.clear_frame(parent).unwrap();
        assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        drop(result);
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        assert_eq!(slots.active_end, 0);
        assert!(slots.slots.iter().all(Option::is_none));
        assert!(slots.slots.capacity() >= original_capacity);
    }

    #[test]
    fn permutations_move_owners_and_failed_insertion_preserves_the_window() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 6;
        let mut slots = SlotStore::new(6);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        for value in [
            Value::Int(0),
            Value::Int(1),
            Value::Int(2),
            Value::Object(object),
            Value::Int(4),
        ] {
            slots.push(&mut window, value).unwrap();
        }
        let capacity = slots.slots.capacity();
        slots.rotate_operands(&window, 1, 4, false).unwrap();
        slots.rotate_operands(&window, 0, 4, true).unwrap();
        slots.insert_copy(&mut window, 4, 5).unwrap();
        assert_eq!(slots.slots.capacity(), capacity);
        assert!(slots.insert_copy(&mut window, 0, 0).is_err());
        assert!(slots.rotate_operands(&window, 1, 6, false).is_err());
        assert!(
            slots
                .rotate_operands(&window, usize::MAX, 2, false)
                .is_err()
        );
        assert_eq!(slots.depth(&window), 6);
        let mut values = slots.take_frame(window).unwrap().operands;
        for expected in [0, 4, 2, 1] {
            assert_eq!(values.pop().unwrap(), Value::Int(expected));
        }
        assert!(
            values
                .iter()
                .all(|value| matches!(value, Value::Object(root) if root.object_id() == id))
        );
        drop(values.pop());
        assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        drop(values);
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        assert_eq!(slots.active_end, 0);
        assert!(slots.slots.iter().all(Option::is_none));
    }

    #[test]
    fn duplicate_sequence_keeps_order_and_independent_object_owners() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 6;
        let mut slots = SlotStore::new(6);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        for value in [Value::Int(7), Value::Object(object), Value::Int(9)] {
            slots.push(&mut window, value).unwrap();
        }
        slots.duplicate_operands(&mut window, 3).unwrap();
        assert!(slots.duplicate_operands(&mut window, 3).is_err());
        assert_eq!(slots.depth(&window), 6);
        for _ in 0..2 {
            assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(9));
            assert!(
                matches!(slots.pop(&mut window).unwrap(), Value::Object(root) if root.object_id() == id)
            );
            assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(7));
        }
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        slots.clear_frame(window).unwrap();
    }

    #[test]
    fn failed_sequence_retain_leaves_committed_prefix_for_driver_cleanup() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 6;
        let mut slots = SlotStore::new(6);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        let text = Value::String(crate::engine::value::JsString::from_static(
            "retained prefix",
        ));
        for value in [text.clone(), Value::Object(object), Value::Int(9)] {
            slots.push(&mut window, value).unwrap();
        }
        {
            // Rc-backed String copies succeed; the following Object retain
            // must fail because it needs a mutable heap borrow (shared
            // borrows take the nested-materialization fast path).
            let state = runtime.0.state.borrow_mut();
            assert!(slots.duplicate_operands(&mut window, 3).is_err());
            assert_eq!(slots.depth(&window), 4);
            assert_eq!(slots.peek(&window, 0).unwrap(), &text);
            assert_eq!(slots.peek(&window, 1).unwrap(), &Value::Int(9));
            assert!(state.heap.object(id).is_ok());
            assert!(!runtime.0.deferred_references.has_pending());
        }
        slots.clear_frame(window).unwrap();
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn cost_collection_distinguishes_reuse_from_initialization_and_live_peaks() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 2;
        let mut slots = SlotStore::new(8);
        let profile = crate::engine::api::profiling::CostProfile::start();
        let mut previous = None;
        for _ in 0..2 {
            let mut window = slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .unwrap();
            slots.push(&mut window, Value::Int(1)).unwrap();
            slots.insert_copy(&mut window, 0, 0).unwrap();
            slots.clear_frame(window).unwrap();
            let cost = profile.snapshot().owned_storage;
            assert_eq!(cost.slot_capacity_growths, 1);
            assert_eq!(cost.maximum_reserved_slots, 2);
            assert_eq!(cost.physical_none_initializations, 2);
            assert_eq!(cost.maximum_initialized_slots, 2);
            assert_eq!(cost.maximum_live_slots, 2);
            if let Some(before) = previous {
                let before: crate::engine::api::profiling::OwnedStorageCost = before;
                assert_eq!(cost.maximum_slot_capacity, before.maximum_slot_capacity);
                assert_eq!(cost.slots_initialized, before.slots_initialized * 2);
                assert_eq!(cost.slot_clears, before.slot_clears * 2);
                assert_eq!(cost.value_copies, before.value_copies * 2);
            }
            previous = Some(cost);
        }
        let before_drop = profile.snapshot();
        drop(slots);
        assert_eq!(
            profile.snapshot(),
            before_drop,
            "cleared arena drop must not count owners twice"
        );
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn late_profile_observes_initialized_backing_without_counting_prior_writes() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 5;
        let mut slots = SlotStore::new(8);
        let warm = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        slots.clear_frame(warm).unwrap();
        owner.metadata.max_stack = 2;
        let profile = crate::engine::api::profiling::CostProfile::start();
        let reused = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let costs = profile.snapshot().owned_storage;
        assert_eq!(costs.physical_none_initializations, 0);
        assert_eq!(costs.maximum_initialized_slots, 5);
        assert_eq!(costs.maximum_reserved_slots, 2);
        assert_eq!(costs.slots_initialized, 2);
        assert_eq!(costs.slot_capacity_growths, 0);
        assert!(costs.maximum_slot_capacity >= 5);
        slots.clear_frame(reused).unwrap();
    }

    #[test]
    fn original_arguments_do_not_alias_writable_parameters() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.argument_count = 2;
        let mut slots = SlotStore::new(32);
        let window = slots
            .push_frame(
                &owner.frame_layout(),
                FrameStorage {
                    original_arguments: vec![Value::Int(1)],
                    parameters: vec![
                        FrameBinding::Direct(Value::Int(1)),
                        FrameBinding::Direct(Value::Undefined),
                    ],
                    ..empty_storage()
                },
            )
            .unwrap();
        let old = slots
            .replace_parameter(&window, 0, FrameBinding::Direct(Value::Int(2)))
            .unwrap();
        assert!(matches!(old, FrameBinding::Direct(Value::Int(1))));
        let storage = slots.take_frame(window).unwrap();
        assert_eq!(storage.original_arguments, vec![Value::Int(1)]);
        assert!(matches!(
            storage.parameters[0],
            FrameBinding::Direct(Value::Int(2))
        ));
        assert!(matches!(
            storage.parameters[1],
            FrameBinding::Direct(Value::Undefined)
        ));
    }

    #[test]
    fn distinct_arenas_reject_matching_numeric_window_ids() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 1;
        let mut first = SlotStore::new(4);
        let mut second = SlotStore::new(4);
        let mut a = first
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        let mut b = second
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        first.push(&mut a, Value::Int(1)).unwrap();
        second.push(&mut b, Value::Int(2)).unwrap();
        assert!(second.peek(&a, 0).is_err());
        assert!(first.peek(&b, 0).is_err());
        assert_eq!(first.pop(&mut a).unwrap(), Value::Int(1));
        assert_eq!(second.pop(&mut b).unwrap(), Value::Int(2));
    }

    #[test]
    fn failed_capacity_and_shape_checks_do_not_change_live_windows() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut owner = PublishedFunctionSnapshot::empty_for_test(context.realm);
        owner.metadata.max_stack = 1;
        let mut slots = SlotStore::new(1);
        let mut window = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        slots.push(&mut window, Value::Int(7)).unwrap();
        assert!(slots.push(&mut window, Value::Int(8)).is_err());
        assert!(
            slots
                .push_frame(&owner.frame_layout(), empty_storage())
                .is_err()
        );
        assert!(
            slots
                .push_frame(
                    &owner.frame_layout(),
                    FrameStorage {
                        locals: vec![FrameBinding::Uninitialized],
                        ..empty_storage()
                    }
                )
                .is_err()
        );
        assert!(slots.peek(&window, usize::MAX).is_err());
        assert_eq!(slots.depth(&window), 1);
        assert_eq!(slots.pop(&mut window).unwrap(), Value::Int(7));
        assert!(slots.pop(&mut window).is_err());
        slots.clear_frame(window).unwrap();
        let reused = slots
            .push_frame(&owner.frame_layout(), empty_storage())
            .unwrap();
        assert_eq!(slots.depth(&reused), 0);
        assert!(slots.slots.iter().all(Option::is_none));
    }
}

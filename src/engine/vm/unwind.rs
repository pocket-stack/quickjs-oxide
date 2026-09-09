use super::*;

impl VmActivation {
    /// Execute QuickJS `OP_append` without publishing a private iterator
    /// region on the frame stack. Unlike ordinary `for-of`, QuickJS closes an
    /// already-created iterator even when its cached `next` path throws while
    /// expanding an Array literal. A close failure cannot replace that
    /// pending throw.
    pub(in crate::engine::vm) fn append_iterable(
        host: &mut impl VmHost,
        array: Value,
        index: Value,
        iterable: Value,
    ) -> Result<OperationOutcome<(Value, Value)>, Error> {
        // `js_append_enumerate` authenticates the private dynamic index before
        // touching the spread source. It then treats the signed Int payload as
        // Uint32 and writes the wrapping result back with JS_NewInt32.
        let Value::Int(raw_index) = index else {
            return Ok(OperationOutcome::Throw(
                host.materialize_error(Error::internal("invalid index for append"))?,
            ));
        };
        let mut position = raw_index as u32;

        // QuickJS keeps the original `sp[-1]` spread source live until the
        // append operation finishes, even when a custom iterator record does
        // not retain it. Preserve that root independently of the iterator.
        let (iterator, next_method, fast_values) = match host.append_start(iterable.clone())? {
            AppendStartOutcome::Record {
                iterator,
                next_method,
                fast_values,
            } => (iterator, next_method, fast_values),
            AppendStartOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };

        if let Some(values) = fast_values {
            for value in values {
                let index = Value::number(f64::from(position));
                if let Completion::Throw(value) =
                    host.define_array_element(array.clone(), index, value)?
                {
                    match host.iterator_close(iterator, true)? {
                        IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                    }
                    return Ok(OperationOutcome::Throw(value));
                }
                position = position.wrapping_add(1);
            }
            return Ok(OperationOutcome::Value((
                array,
                Value::Int(position as i32),
            )));
        }

        loop {
            let value = match host.for_of_next(iterator.clone(), next_method.clone())? {
                ForOfNextOutcome::Result { done: true, .. } => {
                    return Ok(OperationOutcome::Value((
                        array,
                        Value::Int(position as i32),
                    )));
                }
                ForOfNextOutcome::Result { value, done: false } => value,
                ForOfNextOutcome::Throw(value) => {
                    match host.iterator_close(iterator, true)? {
                        IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                    }
                    return Ok(OperationOutcome::Throw(value));
                }
            };

            let index = Value::number(f64::from(position));
            match host.define_array_element(array.clone(), index, value)? {
                Completion::Return(_) => {}
                Completion::Throw(value) => {
                    match host.iterator_close(iterator, true)? {
                        IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                    }
                    return Ok(OperationOutcome::Throw(value));
                }
            }
            position = position.wrapping_add(1);
        }
    }

    pub(in crate::engine::vm) fn raise(
        &mut self,
        value: Value,
        host: &mut impl VmHost,
        code_len: usize,
    ) -> Result<Option<Completion>, Error> {
        host.ensure_backtrace(&value)?;
        loop {
            let Some(region) = self.regions.pop() else {
                return Ok(Some(Completion::Throw(value)));
            };
            match region {
                VmUnwindRegion::Catch {
                    target,
                    stack_depth,
                } => {
                    host.prepare_captured_local_reuse()?;
                    if self.stack.len() < stack_depth {
                        return Err(Error::internal(
                            "exception handler stack depth exceeds the VM stack",
                        ));
                    }
                    self.stack.truncate(stack_depth);
                    self.stack.push(value);
                    self.pc = checked_target(
                        u32::try_from(target)
                            .map_err(|_| Error::internal("catch target overflow"))?,
                        code_len,
                    )?;
                    return Ok(None);
                }
                VmUnwindRegion::Iterator {
                    record_base,
                    enabled,
                    ..
                } => {
                    let required_depth = record_base
                        .checked_add(2)
                        .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
                    if self.stack.len() < required_depth {
                        return Err(Error::internal(
                            "iterator unwind region exceeds the VM stack",
                        ));
                    }
                    let iterator = self.stack[record_base].clone();
                    self.stack.truncate(record_base);
                    if enabled {
                        // IteratorClose with a pending throw never replaces
                        // that throw, including when `return` lookup/call
                        // itself throws. Engine invariant failures still
                        // escape through `Err`.
                        match host.iterator_close(iterator, true)? {
                            IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                        }
                    }
                }
            }
        }
    }

    pub(in crate::engine::vm) fn disable_iterator_region(
        &mut self,
        record_base: usize,
    ) -> Result<(), Error> {
        self.set_iterator_region_enabled(record_base, false, false)?;
        let iterator = self
            .stack
            .get_mut(record_base)
            .ok_or_else(|| Error::internal("iterator record is truncated"))?;
        *iterator = Value::Undefined;
        Ok(())
    }

    pub(in crate::engine::vm) fn set_iterator_region_enabled(
        &mut self,
        record_base: usize,
        enabled_value: bool,
        asynchronous: bool,
    ) -> Result<(), Error> {
        let Some(VmUnwindRegion::Iterator {
            record_base: active_base,
            enabled,
            asynchronous: active_asynchronous,
        }) = self.regions.last_mut()
        else {
            return Err(Error::internal(
                "iterator operation has no innermost iterator region",
            ));
        };
        if *active_base != record_base {
            return Err(Error::internal(
                "iterator unwind region changed during operation",
            ));
        }
        if *active_asynchronous != asynchronous {
            return Err(Error::internal(
                "iterator operation targeted the wrong record family",
            ));
        }
        *enabled = enabled_value;
        Ok(())
    }

    /// Remove the innermost iterator region. When `preserve_top` is true, the
    /// top operand replaces the complete iterator record and all intermediate
    /// values, matching an abrupt return crossing a for-of loop.
    pub(in crate::engine::vm) fn take_iterator_region(
        &mut self,
        preserve_top: bool,
        operation: &'static str,
    ) -> Result<(Value, bool, bool), Error> {
        let region = self
            .regions
            .pop()
            .ok_or_else(|| Error::internal(format!("{operation} has no iterator region")))?;
        let VmUnwindRegion::Iterator {
            record_base,
            enabled,
            asynchronous,
        } = region
        else {
            return Err(Error::internal(format!(
                "{operation} did not target the innermost unwind region"
            )));
        };
        let record_end = record_base
            .checked_add(2)
            .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
        if preserve_top {
            if self.stack.len() <= record_end {
                return Err(Error::internal(format!(
                    "{operation} has no value above its iterator marker"
                )));
            }
        } else if self.stack.len() != record_end {
            return Err(Error::internal(format!(
                "{operation} did not reach its iterator record"
            )));
        }
        let iterator = self
            .stack
            .get(record_base)
            .cloned()
            .ok_or_else(|| Error::internal("iterator record is truncated"))?;
        let preserved = preserve_top.then(|| self.stack.pop()).flatten();
        self.stack.truncate(record_base);
        if let Some(value) = preserved {
            self.stack.push(value);
        }
        Ok((iterator, enabled, asynchronous))
    }
}

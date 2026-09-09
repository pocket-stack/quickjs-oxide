use super::*;

impl Heap {
    /// Store the two arbitrary arguments supplied to a NewPromiseCapability
    /// executor.  Callability is deliberately checked later by the runtime,
    /// after the custom constructor returns, as required by the specification.
    ///
    /// Object edges are retained before publication.  Symbol atoms must be
    /// pre-owned by the caller and transfer to the capture only when this
    /// returns `true`. `false` reports the spec-visible repeated invocation;
    /// the runtime must throw a TypeError and retain caller ownership.
    pub(crate) fn set_promise_capability_capture(
        &mut self,
        id: ObjectId,
        resolve: RawValue,
        reject: RawValue,
    ) -> Result<bool, HeapError> {
        if !is_promise_storable_value(&resolve) || !is_promise_storable_value(&reject) {
            return Err(HeapError::Invariant(
                "Promise capability capture contains an internal value sentinel",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::PromiseCapabilityExecutor,
                        ..
                    },
                internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            } if capture
                .resolve
                .as_ref()
                .is_none_or(|value| matches!(value, RawValue::Undefined))
                && capture
                    .reject
                    .as_ref()
                    .is_none_or(|value| matches!(value, RawValue::Undefined)) => {}
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::PromiseCapabilityExecutor,
                        ..
                    },
                internal: Some(InternalCallableData::PromiseCapabilityExecutor(_)),
            } => {
                return Ok(false);
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Promise capability capture reached the wrong native function",
                ));
            }
        }

        let mut edges = raw_value_edges(&resolve);
        edges.extend(raw_value_edges(&reject));
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::NativeFunction {
            internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("Promise capability executor was validated before retaining arguments")
        };
        capture.resolve = Some(resolve);
        capture.reject = Some(reject);
        Ok(true)
    }

    /// Borrow a copy of the current NewPromiseCapability capture.
    /// Raw values in the result do not own additional heap or atom references.
    pub(crate) fn promise_capability_capture(
        &self,
        id: ObjectId,
    ) -> Result<PromiseCapabilityExecutorData, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::PromiseCapabilityExecutor,
                        ..
                    },
                internal: Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
            } => Ok(capture.clone()),
            _ => Err(HeapError::Invariant(
                "Promise capability lookup reached the wrong native function",
            )),
        }
    }

    /// Borrow a complete snapshot of one genuine Promise's hidden state.
    /// Raw edges in the clone are not independently retained.
    pub(crate) fn promise_snapshot(&self, id: ObjectId) -> Result<PromiseData, HeapError> {
        let ObjectPayload::Promise(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Promise snapshot reached an object with the wrong class",
            ));
        };
        Ok(data.clone())
    }

    /// Append the paired reactions created by one `PerformPromiseThen` call.
    /// Every handler and present capability identity is retained
    /// transactionally before either vector becomes observable to the
    /// collector.
    pub(crate) fn promise_add_reactions(
        &mut self,
        id: ObjectId,
        fulfill: PromiseReaction,
        reject: PromiseReaction,
    ) -> Result<(), HeapError> {
        if fulfill.kind != PromiseReactionKind::Fulfill
            || reject.kind != PromiseReactionKind::Reject
        {
            return Err(HeapError::Invariant(
                "Promise reactions were appended to the wrong settlement lists",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Promise(PromiseData {
                state: PromiseState::Pending,
                ..
            }) => {}
            ObjectPayload::Promise(_) => {
                return Err(HeapError::Invariant(
                    "cannot append reactions to a settled Promise",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Promise reaction append reached an object with the wrong class",
                ));
            }
        }

        let mut edges = promise_reaction_edges(&fulfill);
        edges.extend(promise_reaction_edges(&reject));
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::Promise(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("Promise payload was validated before retaining reactions")
        };
        data.fulfill_reactions.push(fulfill);
        data.reject_reactions.push(reject);
        Ok(())
    }

    /// Settle one pending Promise and detach all pending reaction ownership.
    ///
    /// The runtime must first snapshot and enqueue the selected reaction list;
    /// job enqueue retains its own edges.  This method then publishes the
    /// settled result, releases both obsolete reaction lists, and returns any
    /// detached Symbol atom ownership through the usual cleanup channel.
    pub(crate) fn promise_settle(
        &mut self,
        id: ObjectId,
        state: PromiseState,
        result: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if state == PromiseState::Pending || !is_promise_storable_value(&result) {
            return Err(HeapError::Invariant(
                "Promise settlement requires a final state and ordinary value",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Promise(PromiseData {
                state: PromiseState::Pending,
                ..
            }) => {}
            ObjectPayload::Promise(_) => {
                return Err(HeapError::Invariant("Promise was settled more than once"));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Promise settlement reached an object with the wrong class",
                ));
            }
        }

        let new_edges = raw_value_edges(&result);
        self.retain_edges_transactionally(&new_edges)?;
        let (previous, fulfill_reactions, reject_reactions) = {
            let ObjectPayload::Promise(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("Promise payload was validated before retaining its result")
            };
            let previous = std::mem::replace(&mut data.result, result);
            data.state = state;
            (
                previous,
                std::mem::take(&mut data.fulfill_reactions),
                std::mem::take(&mut data.reject_reactions),
            )
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        for reaction in fulfill_reactions.iter().chain(&reject_reactions) {
            for edge in promise_reaction_edges(reaction) {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Mark one genuine Promise handled and report whether it was already
    /// handled, allowing the runtime to mirror QuickJS rejection tracking.
    pub(crate) fn promise_mark_handled(&mut self, id: ObjectId) -> Result<bool, HeapError> {
        let ObjectPayload::Promise(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Promise handled update reached an object with the wrong class",
            ));
        };
        Ok(std::mem::replace(&mut data.is_handled, true))
    }
}

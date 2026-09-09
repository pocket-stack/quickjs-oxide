use super::*;

impl Heap {
    /// Clone one generator's raw dormant activation while its object still
    /// owns every referenced edge. The runtime immediately maps this snapshot
    /// to rooted handles before beginning the destructive resume transition.
    pub fn generator_snapshot(
        &self,
        id: ObjectId,
    ) -> Result<(GeneratorState, Option<GeneratorActivationData>), HeapError> {
        let ObjectPayload::Generator { state, activation } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Generator state requested for an object with the wrong class",
            ));
        };
        Ok((*state, activation.as_deref().cloned()))
    }

    /// Move a suspended generator to `Executing` and detach the heap-owned
    /// activation edges. The caller must already have rooted a snapshot of the
    /// returned activation so releasing these occurrences cannot invalidate
    /// the active Rust representation.
    pub fn begin_generator_resume(
        &mut self,
        id: ObjectId,
    ) -> Result<(GeneratorState, GeneratorActivationData, HeapCleanup), HeapError> {
        let (state, activation) = {
            let ObjectPayload::Generator { state, activation } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Generator resume reached an object with the wrong class",
                ));
            };
            if !matches!(
                state,
                GeneratorState::SuspendedStart
                    | GeneratorState::SuspendedYield
                    | GeneratorState::SuspendedYieldStar
            ) {
                return Err(HeapError::Invariant(
                    "Generator resume began outside a suspended state",
                ));
            }
            let previous = *state;
            let activation = activation.take().ok_or(HeapError::Invariant(
                "suspended Generator has no activation",
            ))?;
            *state = GeneratorState::Executing;
            (previous, *activation)
        };
        let edges = generator_activation_edges(&activation);
        let atoms = generator_activation_atoms(&activation);
        for edge in edges {
            self.release_raw_no_drain(edge)?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup.atoms.extend(atoms);
        Ok((state, activation, cleanup))
    }

    /// Reattach a fully encoded dormant frame after an executing generator
    /// reaches its next suspension point. Atom occurrences are retained by the
    /// runtime before this call; arena edges are retained transactionally here.
    pub fn suspend_generator(
        &mut self,
        id: ObjectId,
        state: GeneratorState,
        activation: GeneratorActivationData,
    ) -> Result<(), HeapError> {
        if !matches!(
            state,
            GeneratorState::SuspendedStart
                | GeneratorState::SuspendedYield
                | GeneratorState::SuspendedYieldStar
        ) {
            return Err(HeapError::Invariant(
                "Generator suspended with a non-suspended state",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Generator {
                state: GeneratorState::Executing,
                activation: None,
            } => {}
            ObjectPayload::Generator { .. } => {
                return Err(HeapError::Invariant(
                    "Generator suspension did not follow an executing state",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Generator suspension reached an object with the wrong class",
                ));
            }
        }
        let mut candidate = self.object(id)?.clone();
        candidate.payload = ObjectPayload::Generator {
            state,
            activation: Some(Box::new(activation.clone())),
        };
        self.validate_object_layout(&candidate)?;
        let edges = generator_activation_edges(&activation);
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::Generator {
            state: current,
            activation: current_activation,
        } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("Generator payload was validated before suspension")
        };
        *current = state;
        *current_activation = Some(Box::new(activation));
        Ok(())
    }

    /// Permanently finish an executing generator after return, throw, or an
    /// abrupt resume failure. Its dormant edges were already detached by
    /// [`Self::begin_generator_resume`].
    pub fn complete_generator(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let ObjectPayload::Generator { state, activation } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Generator completion reached an object with the wrong class",
            ));
        };
        if *state != GeneratorState::Executing || activation.is_some() {
            return Err(HeapError::Invariant(
                "Generator completion did not follow an executing state",
            ));
        }
        *state = GeneratorState::Completed;
        Ok(())
    }

    pub(crate) fn async_generator_snapshot(
        &self,
        id: ObjectId,
    ) -> Result<AsyncGeneratorData, HeapError> {
        let ObjectPayload::AsyncGenerator(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "AsyncGenerator state requested for an object with the wrong class",
            ));
        };
        Ok(data.clone())
    }

    /// Append one request after the runtime has retained any Symbol atom in
    /// `result`. Arena edges transfer transactionally into the FIFO.
    pub(crate) fn async_generator_enqueue(
        &mut self,
        id: ObjectId,
        request: AsyncGeneratorRequestData,
    ) -> Result<(), HeapError> {
        if !is_promise_storable_value(&request.result) {
            return Err(HeapError::Invariant(
                "AsyncGenerator request contains an internal value sentinel",
            ));
        }
        if !matches!(self.object(id)?.payload, ObjectPayload::AsyncGenerator(_)) {
            return Err(HeapError::Invariant(
                "AsyncGenerator request reached an object with the wrong class",
            ));
        }
        let edges = async_generator_request_edges(&request);
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("AsyncGenerator payload was validated before queue append")
        };
        data.queue.push_back(request);
        Ok(())
    }

    pub(crate) fn async_generator_front_request(
        &self,
        id: ObjectId,
    ) -> Result<Option<AsyncGeneratorRequestData>, HeapError> {
        let ObjectPayload::AsyncGenerator(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "AsyncGenerator request lookup reached an object with the wrong class",
            ));
        };
        Ok(data.queue.front().cloned())
    }

    /// Detach the request which the runtime has already promoted to rooted
    /// values and callables.
    pub(crate) fn async_generator_pop_front(
        &mut self,
        id: ObjectId,
    ) -> Result<(AsyncGeneratorRequestData, HeapCleanup), HeapError> {
        let request = {
            let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "AsyncGenerator request removal reached an object with the wrong class",
                ));
            };
            data.queue.pop_front().ok_or(HeapError::Invariant(
                "AsyncGenerator request queue is empty",
            ))?
        };
        for edge in async_generator_request_edges(&request) {
            self.release_raw_no_drain(edge)?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup.atoms.extend(raw_value_atom(&request.result));
        Ok((request, cleanup))
    }

    /// Move one parked activation into transient rooted runtime ownership.
    pub(crate) fn begin_async_generator_resume(
        &mut self,
        id: ObjectId,
    ) -> Result<(AsyncGeneratorState, GeneratorActivationData, HeapCleanup), HeapError> {
        let (previous, activation, resume_realm) = {
            let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "AsyncGenerator resume reached an object with the wrong class",
                ));
            };
            if !matches!(
                data.state,
                AsyncGeneratorState::SuspendedStart
                    | AsyncGeneratorState::SuspendedYield
                    | AsyncGeneratorState::SuspendedYieldStar
                    | AsyncGeneratorState::Executing
            ) {
                return Err(HeapError::Invariant(
                    "AsyncGenerator resume began outside a parked state",
                ));
            }
            let previous = data.state;
            let activation = data.activation.take().ok_or(HeapError::Invariant(
                "parked AsyncGenerator has no activation",
            ))?;
            let resume_realm = data.resume_realm.take();
            data.state = AsyncGeneratorState::Executing;
            (previous, *activation, resume_realm)
        };
        for edge in generator_activation_edges(&activation) {
            self.release_raw_no_drain(edge)?;
        }
        if let Some(realm) = resume_realm {
            self.release_raw_no_drain(RawId::Context(realm))?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup
            .atoms
            .extend(generator_activation_atoms(&activation));
        Ok((previous, activation, cleanup))
    }

    /// Store a yielded or awaited activation after an executing pump step.
    pub(crate) fn suspend_async_generator(
        &mut self,
        id: ObjectId,
        state: AsyncGeneratorState,
        activation: GeneratorActivationData,
        resume_realm: Option<ContextId>,
    ) -> Result<(), HeapError> {
        if !matches!(
            (state, resume_realm),
            (AsyncGeneratorState::SuspendedYield, None)
                | (AsyncGeneratorState::SuspendedYieldStar, None)
                | (AsyncGeneratorState::Executing, Some(_))
        ) {
            return Err(HeapError::Invariant(
                "AsyncGenerator suspension has an invalid state/realm pair",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::AsyncGenerator(AsyncGeneratorData {
                state: AsyncGeneratorState::Executing,
                activation: None,
                resume_realm: None,
                queue,
            }) if !queue.is_empty() => {}
            ObjectPayload::AsyncGenerator(_) => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator suspension did not follow execution",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator suspension reached an object with the wrong class",
                ));
            }
        }

        let mut candidate = self.object(id)?.clone();
        let ObjectPayload::AsyncGenerator(candidate_data) = &mut candidate.payload else {
            unreachable!("AsyncGenerator payload was validated before suspension")
        };
        candidate_data.state = state;
        candidate_data.activation = Some(Box::new(activation.clone()));
        candidate_data.resume_realm = resume_realm;
        self.validate_object_layout(&candidate)?;

        let mut edges = generator_activation_edges(&activation);
        edges.extend(resume_realm.map(RawId::Context));
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("AsyncGenerator payload was validated before retaining activation")
        };
        data.state = state;
        data.activation = Some(Box::new(activation));
        data.resume_realm = resume_realm;
        Ok(())
    }

    /// Discard any parked activation and enter the absorbing completed state.
    pub(crate) fn complete_async_generator(
        &mut self,
        id: ObjectId,
    ) -> Result<HeapCleanup, HeapError> {
        let (activation, resume_realm) = {
            let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "AsyncGenerator completion reached an object with the wrong class",
                ));
            };
            if matches!(
                data.state,
                AsyncGeneratorState::AwaitingReturn | AsyncGeneratorState::Completed
            ) {
                return Err(HeapError::Invariant(
                    "AsyncGenerator completion repeated or interrupted completed-return await",
                ));
            }
            data.state = AsyncGeneratorState::Completed;
            (data.activation.take(), data.resume_realm.take())
        };
        let mut atoms = Vec::new();
        if let Some(activation) = activation.as_deref() {
            for edge in generator_activation_edges(activation) {
                self.release_raw_no_drain(edge)?;
            }
            atoms.extend(generator_activation_atoms(activation));
        }
        if let Some(realm) = resume_realm {
            self.release_raw_no_drain(RawId::Context(realm))?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup.atoms.extend(atoms);
        Ok(cleanup)
    }

    pub(crate) fn begin_async_generator_completed_return(
        &mut self,
        id: ObjectId,
        realm: ContextId,
    ) -> Result<(), HeapError> {
        self.context(realm)?;
        match &self.object(id)?.payload {
            ObjectPayload::AsyncGenerator(AsyncGeneratorData {
                state: AsyncGeneratorState::Completed,
                activation: None,
                resume_realm: None,
                queue,
            }) if !queue.is_empty() => {}
            ObjectPayload::AsyncGenerator(_) => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator completed return began in an invalid state",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "AsyncGenerator completed return reached the wrong class",
                ));
            }
        }
        self.retain_raw(RawId::Context(realm), 1)?;
        let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("AsyncGenerator payload was validated before completed return")
        };
        data.state = AsyncGeneratorState::AwaitingReturn;
        data.resume_realm = Some(realm);
        Ok(())
    }

    pub(crate) fn finish_async_generator_completed_return(
        &mut self,
        id: ObjectId,
    ) -> Result<HeapCleanup, HeapError> {
        let realm = {
            let ObjectPayload::AsyncGenerator(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "AsyncGenerator completed-return callback reached the wrong class",
                ));
            };
            if data.state != AsyncGeneratorState::AwaitingReturn || data.activation.is_some() {
                return Err(HeapError::Invariant(
                    "AsyncGenerator completed-return callback reached an invalid state",
                ));
            }
            data.state = AsyncGeneratorState::Completed;
            data.resume_realm.take().ok_or(HeapError::Invariant(
                "AsyncGenerator completed-return callback lost its realm",
            ))?
        };
        self.release_raw_no_drain(RawId::Context(realm))?;
        self.drain_zero_queue()
    }

    /// Clone one async driver state while its hidden object still owns every
    /// referenced edge. The runtime must promote any raw identities it keeps
    /// across a subsequent heap mutation.
    pub(crate) fn async_function_state_snapshot(
        &self,
        id: ObjectId,
    ) -> Result<AsyncFunctionStateData, HeapError> {
        let ObjectPayload::AsyncFunctionState(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "AsyncFunction state requested for an object with the wrong class",
            ));
        };
        Ok(data.clone())
    }

    /// Transfer a newly suspended async VM frame into the hidden driver.
    ///
    /// Atom occurrences are pre-owned by the runtime. Arena edges are retained
    /// transactionally before the phase becomes observable as `Awaiting`.
    pub(crate) fn suspend_async_function(
        &mut self,
        id: ObjectId,
        activation: GeneratorActivationData,
    ) -> Result<(), HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::AsyncFunctionState(AsyncFunctionStateData {
                phase: AsyncFunctionPhase::Executing,
                activation: None,
                ..
            }) => {}
            ObjectPayload::AsyncFunctionState(_) => {
                return Err(HeapError::Invariant(
                    "AsyncFunction suspension did not follow an executing phase",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "AsyncFunction suspension reached an object with the wrong class",
                ));
            }
        }

        let mut candidate = self.object(id)?.clone();
        let ObjectPayload::AsyncFunctionState(candidate_data) = &mut candidate.payload else {
            unreachable!("AsyncFunction payload was validated before suspension")
        };
        candidate_data.phase = AsyncFunctionPhase::Awaiting;
        candidate_data.activation = Some(Box::new(activation.clone()));
        self.validate_object_layout(&candidate)?;

        let edges = generator_activation_edges(&activation);
        self.retain_edges_transactionally(&edges)?;
        let ObjectPayload::AsyncFunctionState(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("AsyncFunction payload was validated before retaining its activation")
        };
        data.phase = AsyncFunctionPhase::Awaiting;
        data.activation = Some(Box::new(activation));
        Ok(())
    }

    /// Move an awaiting async driver back to `Executing` and detach its
    /// heap-owned activation. The caller must first root the snapshot it will
    /// execute so releasing the dormant occurrences remains safe.
    pub(crate) fn begin_async_function_resume(
        &mut self,
        id: ObjectId,
    ) -> Result<(GeneratorActivationData, HeapCleanup), HeapError> {
        let activation = {
            let ObjectPayload::AsyncFunctionState(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "AsyncFunction resume reached an object with the wrong class",
                ));
            };
            if data.phase != AsyncFunctionPhase::Awaiting {
                return Err(HeapError::Invariant(
                    "AsyncFunction resume began outside an awaiting phase",
                ));
            }
            let activation = data.activation.take().ok_or(HeapError::Invariant(
                "awaiting AsyncFunction has no activation",
            ))?;
            data.phase = AsyncFunctionPhase::Executing;
            *activation
        };

        let edges = generator_activation_edges(&activation);
        let atoms = generator_activation_atoms(&activation);
        for edge in edges {
            self.release_raw_no_drain(edge)?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup.atoms.extend(atoms);
        Ok((activation, cleanup))
    }

    /// Permanently finish an async driver after resolving or rejecting its
    /// outer promise. Failure after an `await` has been published may complete
    /// directly from `Awaiting`; in that case dormant frame ownership is
    /// detached and returned through the ordinary cleanup channel.
    pub(crate) fn complete_async_function(
        &mut self,
        id: ObjectId,
    ) -> Result<HeapCleanup, HeapError> {
        let activation = {
            let ObjectPayload::AsyncFunctionState(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "AsyncFunction completion reached an object with the wrong class",
                ));
            };
            let activation = match data.phase {
                AsyncFunctionPhase::Executing if data.activation.is_none() => None,
                AsyncFunctionPhase::Awaiting if data.activation.is_some() => {
                    let Some(activation) = data.activation.take() else {
                        unreachable!("activation presence was checked")
                    };
                    Some(*activation)
                }
                AsyncFunctionPhase::Executing
                | AsyncFunctionPhase::Awaiting
                | AsyncFunctionPhase::Completed => {
                    return Err(HeapError::Invariant(
                        "AsyncFunction completion reached an inconsistent phase",
                    ));
                }
            };
            data.phase = AsyncFunctionPhase::Completed;
            activation
        };

        let Some(activation) = activation else {
            return Ok(HeapCleanup::default());
        };
        let edges = generator_activation_edges(&activation);
        let atoms = generator_activation_atoms(&activation);
        for edge in edges {
            self.release_raw_no_drain(edge)?;
        }
        let mut cleanup = self.drain_zero_queue()?;
        cleanup.atoms.extend(atoms);
        Ok(cleanup)
    }
}

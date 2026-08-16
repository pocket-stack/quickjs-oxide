//! Authentication and traversal planning for the canonical bytecode-image writer.
//!
//! The writer first builds a source-bound plan which validates traversal,
//! resource, atom, and code-sidecar invariants without exposing output. Only a
//! complete plan may allocate the final BC5 byte vector. Nothing in this
//! module materializes a runtime object or admits native QuickJS code to
//! execution.

use std::collections::{HashMap, HashSet};

use crate::atom::ATOM_MAX_INT;

use super::super::super::atoms::{AtomIndexSpace, BinaryObjectMode};
use super::super::super::code::{CodeError, CodeResourceKind};
use super::super::super::function_envelope::{
    FunctionEnvelopeError, FunctionEnvelopeLimits, FunctionField, FunctionResourceKind,
};
use super::super::super::graph::model::{
    AtomId, GraphError, GraphResourceKind, NodeId, TypedArrayBackingError, TypedArrayKind,
    WireNodeCarrier, WirePropertyCarrier, WireValue, canonical_bigint_length,
    validate_array_buffer_layout, validate_typed_array_write_layout,
};
use super::super::super::graph::write_state::{DataNodeWrite, DataWriteState};
use super::super::super::pinned_atoms::PinnedAtomKind;
use super::super::super::wire::{BcTag, ResourceKind, WireError, WireString};
use super::super::atoms::{ImageAtom, ImageKey};
use super::super::budget::{
    BytecodeImageBudgetError, BytecodeImageResourceKind, FunctionTotals, FunctionUsage,
};
use super::super::model::{
    BytecodeImage, FunctionId, FunctionRecord, ImageCode, ImageFunctionEnvelope, ImageValue,
};
use super::emit::encoded_plan_length;
use super::{BytecodeImageEncodeError, BytecodeImageEncodeOptions};

const FUNCTION_FLAGS_MASK: u16 = 0x0fff;
const FUNCTION_HAS_DEBUG: u16 = 1 << 10;
const CLOSURE_FLAGS_MASK: u16 = 0x01ff;
const MAX_QUICKJS_POSITIVE_INT: usize = i32::MAX as usize;

#[derive(Clone, Copy)]
pub(super) enum PlannedToken<'a> {
    U8(u8),
    U16(u16),
    Uleb(u32),
    I32(i32),
    F64(u64),
    String(&'a WireString),
    Bytes(&'a [u8]),
    Atom(ImageAtom),
    Code {
        function: FunctionId,
        code: &'a ImageCode,
    },
}

#[derive(Clone, Copy)]
enum ValueRef<'a> {
    Image(&'a ImageValue),
    Wire(&'a WireValue),
    Node(NodeId),
}

enum PlanTask<'a> {
    Value {
        value: ValueRef<'a>,
        whole_parent_depth: usize,
        graph_parent_depth: usize,
    },
    Key(ImageKey),
    LeaveNode(NodeId),
    LeaveFunction(FunctionId),
}

/// Move-only proof that every byte-affecting invariant was checked against one
/// borrowed image. The proof is private and consumed by final emission.
pub(super) struct AuthenticatedBytecodeImage<'a> {
    pub(super) image: &'a BytecodeImage,
    pub(super) options: BytecodeImageEncodeOptions,
    pub(super) atoms: Vec<&'a WireString>,
    pub(super) dynamic_slots: HashMap<AtomId, u32>,
    pub(super) atom_space: AtomIndexSpace,
    pub(super) tokens: Vec<PlannedToken<'a>>,
    pub(super) encoded_length: usize,
}

struct PlanBuilder<'a> {
    image: &'a BytecodeImage,
    options: BytecodeImageEncodeOptions,
    atoms: Vec<&'a WireString>,
    dynamic_slots: HashMap<AtomId, u32>,
    tokens: Vec<PlannedToken<'a>>,
    tasks: Vec<PlanTask<'a>>,
    data_state: DataWriteState,
    active_functions: HashSet<FunctionId>,
    seen_functions: HashSet<FunctionId>,
    emitted_functions: usize,
    function_totals: FunctionTotals,
}

pub(super) fn authenticate_for_write<'a>(
    image: &'a BytecodeImage,
    options: BytecodeImageEncodeOptions,
) -> Result<AuthenticatedBytecodeImage<'a>, BytecodeImageEncodeError> {
    PlanBuilder::new(image, options)?.authenticate()
}

impl<'a> PlanBuilder<'a> {
    fn new(
        image: &'a BytecodeImage,
        options: BytecodeImageEncodeOptions,
    ) -> Result<Self, BytecodeImageEncodeError> {
        let mut tasks = Vec::new();
        tasks
            .try_reserve(1)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
        tasks.push(PlanTask::Value {
            value: ValueRef::Image(image.root()),
            whole_parent_depth: 0,
            graph_parent_depth: 0,
        });
        Ok(Self {
            image,
            options,
            atoms: Vec::new(),
            dynamic_slots: HashMap::new(),
            tokens: Vec::new(),
            tasks,
            data_state: DataWriteState::new(
                options.limits.graph(),
                options.allow_object_references,
            ),
            active_functions: HashSet::new(),
            seen_functions: HashSet::new(),
            emitted_functions: 0,
            function_totals: FunctionTotals::default(),
        })
    }

    fn authenticate(mut self) -> Result<AuthenticatedBytecodeImage<'a>, BytecodeImageEncodeError> {
        while let Some(task) = self.tasks.pop() {
            match task {
                PlanTask::Value {
                    value,
                    whole_parent_depth,
                    graph_parent_depth,
                } => self.plan_value(value, whole_parent_depth, graph_parent_depth)?,
                PlanTask::Key(key) => self.plan_atom(key_atom(key))?,
                PlanTask::LeaveNode(node) => self.data_state.leave_node(node),
                PlanTask::LeaveFunction(function) => {
                    if !self.active_functions.remove(&function) {
                        return Err(BytecodeImageEncodeError::CircularFunction {
                            function_index: function.zero_based(),
                        });
                    }
                }
            }
        }

        if self.seen_functions.len() != self.image.functions().len() {
            return Err(BytecodeImageEncodeError::MissingFunctions {
                reachable: self.seen_functions.len(),
                function_count: self.image.functions().len(),
            });
        }

        let atom_count = u32::try_from(self.atoms.len()).map_err(|_| {
            BytecodeImageEncodeError::DynamicAtomOutOfRange {
                index: u32::MAX,
                atom_count: self.atoms.len(),
            }
        })?;
        let atom_space = AtomIndexSpace::new(BinaryObjectMode::Bytecode, atom_count)?;
        let encoded_length =
            encoded_plan_length(&self.atoms, &self.dynamic_slots, atom_space, &self.tokens)?;
        if encoded_length > self.options.max_output_bytes {
            return Err(WireError::ResourceLimit {
                kind: ResourceKind::OutputBytes,
                requested: encoded_length,
                limit: self.options.max_output_bytes,
            }
            .into());
        }

        Ok(AuthenticatedBytecodeImage {
            image: self.image,
            options: self.options,
            atoms: self.atoms,
            dynamic_slots: self.dynamic_slots,
            atom_space,
            tokens: self.tokens,
            encoded_length,
        })
    }

    fn plan_value(
        &mut self,
        value: ValueRef<'a>,
        whole_parent_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let whole_depth =
            whole_parent_depth
                .checked_add(1)
                .ok_or(BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::WholeDepth,
                })?;
        self.options
            .limits
            .check(BytecodeImageResourceKind::WholeDepth, whole_depth)?;

        match value {
            ValueRef::Image(value) => match value.as_wire() {
                Ok(value) => self.plan_wire_value(value, whole_depth, graph_parent_depth),
                Err(function) => self.plan_function(function, whole_depth, graph_parent_depth),
            },
            ValueRef::Wire(value) => self.plan_wire_value(value, whole_depth, graph_parent_depth),
            ValueRef::Node(node) => self.plan_node(node, whole_depth, graph_parent_depth),
        }
    }

    fn plan_wire_value(
        &mut self,
        value: &'a WireValue,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        match value {
            WireValue::Undefined => self.push_u8(BcTag::Undefined.to_byte()),
            WireValue::Null => self.push_u8(BcTag::Null.to_byte()),
            WireValue::Bool(false) => self.push_u8(BcTag::BoolFalse.to_byte()),
            WireValue::Bool(true) => self.push_u8(BcTag::BoolTrue.to_byte()),
            WireValue::Int32(value) => {
                self.push_u8(BcTag::Int32.to_byte())?;
                self.push_token(PlannedToken::I32(*value))
            }
            WireValue::Float64Bits(bits) => {
                self.push_u8(BcTag::Float64.to_byte())?;
                self.push_token(PlannedToken::F64(*bits))
            }
            WireValue::String(value) => {
                self.push_u8(BcTag::String.to_byte())?;
                self.push_token(PlannedToken::String(value))
            }
            WireValue::BigInt(payload) => {
                self.charge_bigint(payload)?;
                self.push_u8(BcTag::BigInt.to_byte())?;
                let length =
                    u32::try_from(payload.len()).map_err(|_| GraphError::CountOverflow {
                        kind: GraphResourceKind::BigIntBytes,
                    })?;
                self.push_uleb(length)?;
                self.push_token(PlannedToken::Bytes(payload))
            }
            WireValue::Node(node) => self.plan_node(*node, whole_depth, graph_parent_depth),
        }
    }

    fn plan_node(
        &mut self,
        node: NodeId,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let nodes = self.image.nodes();
        let node_data = nodes
            .get(node.as_usize())
            .ok_or(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count: nodes.len(),
            })?;

        match self.data_state.enter_node(node)? {
            DataNodeWrite::Reference(index) => {
                self.push_u8(BcTag::ObjectReference.to_byte())?;
                return self.push_uleb(index);
            }
            DataNodeWrite::Traverse => {}
        }

        let graph_depth = self.data_state.child_depth(graph_parent_depth)?;

        // Resource preflight precedes duplicate-key scratch allocation and
        // structural validation, matching the bounded graph writer. Ordinary
        // objects charge only string-key properties because pinned QuickJS
        // silently omits enumerable symbol and private-name properties.
        let ordinary_property_count = match node_data {
            WireNodeCarrier::Ordinary { properties } => {
                let count = properties
                    .iter()
                    .filter(|property| is_string_property_key(property.key))
                    .count();
                self.charge_container(count)?;
                Some(count)
            }
            WireNodeCarrier::Array { elements }
            | WireNodeCarrier::TemplateObject { elements, .. } => {
                self.charge_container(elements.len())?;
                None
            }
            WireNodeCarrier::ArrayBuffer { bytes, .. } => {
                self.charge_array_buffer(bytes.len())?;
                None
            }
            WireNodeCarrier::TypedArray { .. }
            | WireNodeCarrier::ObjectValue { .. }
            | WireNodeCarrier::Date { .. } => None,
        };

        if let Some(reservation) = self.data_state.reserve_unique_node(node)? {
            match node_data {
                WireNodeCarrier::Ordinary { properties } => {
                    validate_properties(node, properties)?;
                }
                WireNodeCarrier::ArrayBuffer {
                    bytes,
                    max_byte_length,
                } => {
                    validate_array_buffer_layout(bytes.len(), *max_byte_length).map_err(
                        |reason| BytecodeImageEncodeError::InvalidArrayBuffer { node, reason },
                    )?;
                }
                WireNodeCarrier::TypedArray {
                    kind,
                    length,
                    byte_offset,
                    buffer,
                } => validate_typed_array(nodes, node, *kind, *length, *byte_offset, *buffer)?,
                WireNodeCarrier::Array { .. }
                | WireNodeCarrier::TemplateObject { .. }
                | WireNodeCarrier::ObjectValue { .. }
                | WireNodeCarrier::Date { .. } => {}
            }
            reservation.commit();
        }

        if !self.data_state.allows_object_references() {
            self.push_task(PlanTask::LeaveNode(node))?;
        }

        match node_data {
            WireNodeCarrier::Ordinary { properties } => {
                let property_count = ordinary_property_count
                    .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)?;
                self.push_u8(BcTag::Object.to_byte())?;
                self.push_uleb(container_count(property_count)?)?;
                self.reserve_tasks(property_count.checked_mul(2))?;
                for property in properties
                    .iter()
                    .rev()
                    .filter(|property| is_string_property_key(property.key))
                {
                    self.tasks.push(PlanTask::Value {
                        value: ValueRef::Image(&property.value),
                        whole_parent_depth: whole_depth,
                        graph_parent_depth: graph_depth,
                    });
                    self.tasks.push(PlanTask::Key(property.key));
                }
            }
            WireNodeCarrier::Array { elements } => {
                self.push_u8(BcTag::Array.to_byte())?;
                self.push_uleb(container_count(elements.len())?)?;
                self.reserve_tasks(Some(elements.len()))?;
                for element in elements.iter().rev() {
                    self.tasks.push(PlanTask::Value {
                        value: ValueRef::Image(element),
                        whole_parent_depth: whole_depth,
                        graph_parent_depth: graph_depth,
                    });
                }
            }
            WireNodeCarrier::TemplateObject { elements, raw } => {
                self.push_u8(BcTag::TemplateObject.to_byte())?;
                self.push_uleb(container_count(elements.len())?)?;
                self.reserve_tasks(elements.len().checked_add(1))?;
                self.tasks.push(PlanTask::Value {
                    value: ValueRef::Image(raw),
                    whole_parent_depth: whole_depth,
                    graph_parent_depth: graph_depth,
                });
                for element in elements.iter().rev() {
                    self.tasks.push(PlanTask::Value {
                        value: ValueRef::Image(element),
                        whole_parent_depth: whole_depth,
                        graph_parent_depth: graph_depth,
                    });
                }
            }
            WireNodeCarrier::ArrayBuffer {
                bytes,
                max_byte_length,
            } => {
                let byte_length =
                    validate_array_buffer_layout(bytes.len(), *max_byte_length).map_err(
                        |reason| BytecodeImageEncodeError::InvalidArrayBuffer { node, reason },
                    )?;
                self.push_u8(BcTag::ArrayBuffer.to_byte())?;
                self.push_uleb(byte_length)?;
                self.push_uleb(max_byte_length.unwrap_or(u32::MAX))?;
                self.push_token(PlannedToken::Bytes(bytes))?;
            }
            WireNodeCarrier::TypedArray {
                kind,
                length,
                byte_offset,
                buffer,
            } => {
                self.push_u8(BcTag::TypedArray.to_byte())?;
                self.push_u8(kind.to_wire_byte())?;
                self.push_uleb(*length)?;
                self.push_uleb(*byte_offset)?;
                self.push_task(PlanTask::Value {
                    value: ValueRef::Node(*buffer),
                    whole_parent_depth: whole_depth,
                    graph_parent_depth: graph_depth,
                })?;
            }
            WireNodeCarrier::ObjectValue { primitive } => {
                self.push_u8(BcTag::ObjectValue.to_byte())?;
                self.push_task(PlanTask::Value {
                    value: ValueRef::Wire(primitive.as_wire_value()),
                    whole_parent_depth: whole_depth,
                    graph_parent_depth: graph_depth,
                })?;
            }
            WireNodeCarrier::Date { time_value } => {
                self.push_u8(BcTag::Date.to_byte())?;
                self.push_task(PlanTask::Value {
                    value: ValueRef::Wire(time_value.as_wire_value()),
                    whole_parent_depth: whole_depth,
                    graph_parent_depth: graph_depth,
                })?;
            }
        }
        Ok(())
    }

    fn plan_function(
        &mut self,
        function: FunctionId,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let record =
            self.image
                .function(function)
                .ok_or(BytecodeImageEncodeError::ForeignFunction {
                    function_index: function.zero_based(),
                })?;
        if self.active_functions.contains(&function) {
            return Err(BytecodeImageEncodeError::CircularFunction {
                function_index: function.zero_based(),
            });
        }
        self.charge_function_occurrence()?;
        if !self.seen_functions.contains(&function) {
            let expected = u32::try_from(self.seen_functions.len()).map_err(|_| {
                BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::Functions,
                }
            })?;
            if function.zero_based() != expected {
                return Err(BytecodeImageEncodeError::FunctionPreorder {
                    expected,
                    found: function.zero_based(),
                });
            }
        }
        self.authenticate_function_record(function, record)?;
        if !self.seen_functions.contains(&function) {
            self.seen_functions
                .try_reserve(1)
                .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
            self.seen_functions.insert(function);
        }
        self.active_functions
            .try_reserve(1)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
        self.active_functions.insert(function);

        self.plan_function_prefix(function, record)?;

        self.push_task(PlanTask::LeaveFunction(function))?;
        self.reserve_tasks(Some(record.constants().len()))?;
        for constant in record.constants().iter().rev() {
            self.tasks.push(PlanTask::Value {
                value: ValueRef::Image(constant),
                whole_parent_depth: whole_depth,
                graph_parent_depth,
            });
        }
        Ok(())
    }

    fn plan_function_prefix(
        &mut self,
        function: FunctionId,
        record: &'a FunctionRecord,
    ) -> Result<(), BytecodeImageEncodeError> {
        let envelope = record.envelope();

        self.push_u8(BcTag::FunctionBytecode.to_byte())?;
        let flags =
            envelope.flags().raw() | (u16::from(envelope.debug().is_some()) * FUNCTION_HAS_DEBUG);
        self.push_token(PlannedToken::U16(flags))?;
        self.push_u8(envelope.js_mode().raw())?;
        self.plan_atom(envelope.name())?;
        self.push_uleb(u32::from(envelope.argument_count()))?;
        self.push_uleb(u32::from(envelope.variable_count()))?;
        self.push_uleb(u32::from(envelope.defined_argument_count()))?;
        self.push_uleb(u32::from(envelope.stack_size()))?;
        self.push_uleb(u32::from(envelope.variable_reference_count()))?;
        self.push_uleb(positive_u32(
            envelope.closures().len(),
            FunctionField::ClosureVariableCount,
        )?)?;
        self.push_uleb(positive_u32(
            record.constants().len(),
            FunctionField::ConstantPoolCount,
        )?)?;
        self.push_uleb(positive_u32(
            envelope.code().as_bytes().len(),
            FunctionField::ByteCodeLength,
        )?)?;
        self.push_uleb(positive_u32(
            envelope.locals().len(),
            FunctionField::LocalCount,
        )?)?;

        for local in envelope.locals() {
            self.plan_atom(local.name())?;
            let encoded =
                local
                    .scope_next()
                    .encode()
                    .ok_or(FunctionEnvelopeError::CountOverflow {
                        field: FunctionField::LocalScopeNext,
                    })?;
            self.push_uleb(encoded)?;
            self.push_uleb(u32::from(local.variable_reference_index()))?;
            self.push_u8(local.flags().raw())?;
        }
        for closure in envelope.closures() {
            self.plan_atom(closure.name())?;
            self.push_uleb(u32::from(closure.variable_index()))?;
            self.push_token(PlannedToken::U16(closure.flags().raw()))?;
        }

        for relocation in envelope.code().atom_relocations() {
            self.encounter_atom(relocation.atom())?;
        }
        self.push_token(PlannedToken::Code {
            function,
            code: envelope.code(),
        })?;

        if let Some(debug) = envelope.debug() {
            self.plan_atom(debug.filename())?;
            self.push_uleb(positive_u32(
                debug.pc2line().len(),
                FunctionField::Pc2LineLength,
            )?)?;
            self.push_token(PlannedToken::Bytes(debug.pc2line()))?;
            self.push_uleb(positive_u32(
                debug.source().len(),
                FunctionField::SourceLength,
            )?)?;
            self.push_token(PlannedToken::Bytes(debug.source()))?;
        }
        Ok(())
    }

    fn plan_atom(&mut self, atom: ImageAtom) -> Result<(), BytecodeImageEncodeError> {
        self.encounter_atom(atom)?;
        self.push_token(PlannedToken::Atom(atom))
    }

    fn encounter_atom(&mut self, atom: ImageAtom) -> Result<(), BytecodeImageEncodeError> {
        match atom {
            ImageAtom::Index(index) if index > ATOM_MAX_INT => {
                Err(BytecodeImageEncodeError::IntegerAtomOutOfRange { index })
            }
            ImageAtom::Dynamic(atom) if !self.dynamic_slots.contains_key(&atom) => {
                let string = self.image.atoms().get(atom.as_usize()).ok_or(
                    BytecodeImageEncodeError::DynamicAtomOutOfRange {
                        index: atom.zero_based(),
                        atom_count: self.image.atoms().len(),
                    },
                )?;
                let slot = u32::try_from(self.atoms.len()).map_err(|_| {
                    BytecodeImageEncodeError::DynamicAtomOutOfRange {
                        index: atom.zero_based(),
                        atom_count: self.image.atoms().len(),
                    }
                })?;
                self.atoms
                    .try_reserve(1)
                    .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
                self.dynamic_slots
                    .try_reserve(1)
                    .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
                self.atoms.push(string);
                self.dynamic_slots.insert(atom, slot);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn charge_function_occurrence(&mut self) -> Result<(), BytecodeImageEncodeError> {
        let requested = self.emitted_functions.checked_add(1).ok_or(
            BytecodeImageBudgetError::CountOverflow {
                kind: BytecodeImageResourceKind::Functions,
            },
        )?;
        self.options
            .limits
            .check(BytecodeImageResourceKind::Functions, requested)?;
        self.emitted_functions = requested;
        Ok(())
    }

    fn authenticate_function_record(
        &mut self,
        function: FunctionId,
        record: &FunctionRecord,
    ) -> Result<(), BytecodeImageEncodeError> {
        let envelope = record.envelope();
        let remaining = self.function_totals.remaining(self.options.limits)?;
        let envelope_limits = remaining.intersect(self.options.limits.envelope());
        if let Err(error) = validate_envelope(envelope, record.constants().len(), envelope_limits) {
            if let Some(error) = self.function_totals.aggregate_error_for_envelope(
                &error,
                remaining,
                self.options.limits,
            ) {
                return Err(error.into());
            }
            return Err(error.into());
        }
        validate_code(function, envelope.code())?;

        let debug_bytes = envelope.debug().map_or(Ok(0), |debug| {
            debug
                .pc2line()
                .len()
                .checked_add(debug.source().len())
                .ok_or(BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::TotalDebugBytes,
                })
        })?;
        let usage = FunctionUsage::new(
            record.constants().len(),
            envelope.locals().len(),
            envelope.closures().len(),
            envelope.code().as_bytes().len(),
            envelope.code().instructions().len(),
            envelope.code().atom_relocations().len(),
            debug_bytes,
        );
        self.function_totals = self
            .function_totals
            .checked_add(usage, self.options.limits)?;
        Ok(())
    }

    fn charge_container(&mut self, entries: usize) -> Result<(), BytecodeImageEncodeError> {
        self.data_state.check_container_entries(entries)?;
        self.data_state.charge_container_entries(entries)?;
        Ok(())
    }

    fn charge_bigint(&mut self, payload: &[u8]) -> Result<(), BytecodeImageEncodeError> {
        self.data_state.check_bigint_bytes(payload.len())?;
        if canonical_bigint_length(payload) != payload.len() {
            return Err(BytecodeImageEncodeError::NonCanonicalBigInt);
        }
        self.data_state.charge_bigint_bytes(payload.len())?;
        Ok(())
    }

    fn charge_array_buffer(&mut self, bytes: usize) -> Result<(), BytecodeImageEncodeError> {
        self.data_state.charge_array_buffer_bytes(bytes)?;
        Ok(())
    }

    fn push_u8(&mut self, value: u8) -> Result<(), BytecodeImageEncodeError> {
        self.push_token(PlannedToken::U8(value))
    }

    fn push_uleb(&mut self, value: u32) -> Result<(), BytecodeImageEncodeError> {
        self.push_token(PlannedToken::Uleb(value))
    }

    fn push_token(&mut self, token: PlannedToken<'a>) -> Result<(), BytecodeImageEncodeError> {
        self.tokens
            .try_reserve(1)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
        self.tokens.push(token);
        Ok(())
    }

    fn push_task(&mut self, task: PlanTask<'a>) -> Result<(), BytecodeImageEncodeError> {
        self.tasks
            .try_reserve(1)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
        self.tasks.push(task);
        Ok(())
    }

    fn reserve_tasks(&mut self, count: Option<usize>) -> Result<(), BytecodeImageEncodeError> {
        let count = count.ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)?;
        self.tasks
            .try_reserve(count)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)
    }
}

fn validate_properties(
    node: NodeId,
    properties: &[WirePropertyCarrier<ImageValue, ImageKey>],
) -> Result<(), BytecodeImageEncodeError> {
    let mut keys = HashSet::new();
    let string_property_count = properties
        .iter()
        .filter(|property| is_string_property_key(property.key))
        .count();
    keys.try_reserve(string_property_count)
        .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
    for property in properties {
        if !is_string_property_key(property.key) {
            continue;
        }
        if !keys.insert(property.key) {
            return Err(BytecodeImageEncodeError::DuplicatePropertyKey { node });
        }
    }
    Ok(())
}

fn is_string_property_key(key: ImageKey) -> bool {
    !matches!(
        key,
        ImageKey::Predefined(atom) if atom.kind() != PinnedAtomKind::String
    )
}

fn validate_typed_array(
    nodes: &[WireNodeCarrier<ImageValue, ImageKey>],
    node: NodeId,
    kind: TypedArrayKind,
    length: u32,
    byte_offset: u32,
    buffer: NodeId,
) -> Result<(), BytecodeImageEncodeError> {
    let backing = nodes
        .get(buffer.as_usize())
        .ok_or(GraphError::InvalidNodeIndex {
            index: buffer.zero_based(),
            node_count: nodes.len(),
        })?;
    let WireNodeCarrier::ArrayBuffer {
        bytes,
        max_byte_length,
    } = backing
    else {
        return Err(BytecodeImageEncodeError::InvalidTypedArrayBacking {
            node,
            reason: TypedArrayBackingError::NotArrayBuffer { node: buffer },
        });
    };
    validate_array_buffer_layout(bytes.len(), *max_byte_length).map_err(|reason| {
        BytecodeImageEncodeError::InvalidArrayBuffer {
            node: buffer,
            reason,
        }
    })?;
    validate_typed_array_write_layout(kind, length, byte_offset, bytes.len(), *max_byte_length)
        .map_err(|reason| BytecodeImageEncodeError::InvalidTypedArray { node, reason })
}

fn validate_envelope(
    envelope: &ImageFunctionEnvelope,
    constant_count: usize,
    function_limits: FunctionEnvelopeLimits,
) -> Result<(), FunctionEnvelopeError> {
    let invalid_flags = envelope.flags().raw() & (!FUNCTION_FLAGS_MASK | FUNCTION_HAS_DEBUG);
    if invalid_flags != 0 {
        return Err(FunctionEnvelopeError::InvalidModelBits {
            field: FunctionField::FunctionFlags,
            bits: invalid_flags,
        });
    }
    let expected_locals = usize::from(envelope.argument_count())
        .checked_add(usize::from(envelope.variable_count()))
        .ok_or(FunctionEnvelopeError::CountOverflow {
            field: FunctionField::LocalCount,
        })?;
    if !envelope.locals().is_empty() && envelope.locals().len() != expected_locals {
        return Err(FunctionEnvelopeError::NonCanonicalLocalTableLength {
            argument_count: envelope.argument_count(),
            variable_count: envelope.variable_count(),
            local_count: envelope.locals().len(),
        });
    }
    for local in envelope.locals() {
        if local.scope_next().encode().is_none() {
            return Err(FunctionEnvelopeError::CountOverflow {
                field: FunctionField::LocalScopeNext,
            });
        }
    }
    for closure in envelope.closures() {
        let invalid = closure.flags().raw() & !CLOSURE_FLAGS_MASK;
        if invalid != 0 {
            return Err(FunctionEnvelopeError::InvalidModelBits {
                field: FunctionField::ClosureFlags,
                bits: invalid,
            });
        }
    }

    check_function_limit(
        function_limits,
        FunctionResourceKind::LocalVariables,
        envelope.locals().len(),
    )?;
    check_function_limit(
        function_limits,
        FunctionResourceKind::ClosureVariables,
        envelope.closures().len(),
    )?;
    check_function_limit(
        function_limits,
        FunctionResourceKind::ConstantPoolEntries,
        constant_count,
    )?;
    check_code_limit(
        function_limits,
        CodeResourceKind::Bytes,
        envelope.code().as_bytes().len(),
    )?;
    check_code_limit(
        function_limits,
        CodeResourceKind::Instructions,
        envelope.code().instructions().len(),
    )?;
    check_code_limit(
        function_limits,
        CodeResourceKind::AtomRelocations,
        envelope.code().atom_relocations().len(),
    )?;
    if let Some(debug) = envelope.debug() {
        check_function_limit(
            function_limits,
            FunctionResourceKind::Pc2LineBytes,
            debug.pc2line().len(),
        )?;
        check_function_limit(
            function_limits,
            FunctionResourceKind::SourceBytes,
            debug.source().len(),
        )?;
        let total = debug
            .pc2line()
            .len()
            .checked_add(debug.source().len())
            .ok_or(FunctionEnvelopeError::CountOverflow {
                field: FunctionField::SourceLength,
            })?;
        check_function_limit(
            function_limits,
            FunctionResourceKind::TotalDebugBytes,
            total,
        )?;
    }
    for (value, field) in [
        (envelope.locals().len(), FunctionField::LocalCount),
        (
            envelope.closures().len(),
            FunctionField::ClosureVariableCount,
        ),
        (constant_count, FunctionField::ConstantPoolCount),
        (
            envelope.code().as_bytes().len(),
            FunctionField::ByteCodeLength,
        ),
    ] {
        let _ = positive_u32(value, field)?;
    }
    Ok(())
}

fn validate_code(function: FunctionId, code: &ImageCode) -> Result<(), BytecodeImageEncodeError> {
    let mut expected_offset = 0usize;
    let mut relocation_index = 0usize;
    for instruction in code.instructions() {
        let offset = instruction.offset() as usize;
        if offset != expected_offset || instruction.opcode().raw() == 0 {
            return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            });
        }
        let size = usize::from(instruction.opcode().size());
        let end = offset
            .checked_add(size)
            .ok_or(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            })?;
        if size == 0 || end > code.as_bytes().len() {
            return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                function_index: function.zero_based(),
                offset: instruction.offset(),
            });
        }
        if let Some(delta) = instruction.opcode().atom_operand_offset() {
            let operand = offset.checked_add(usize::from(delta)).ok_or(
                BytecodeImageEncodeError::InvalidCodeSidecar {
                    function_index: function.zero_based(),
                    offset: instruction.offset(),
                },
            )?;
            let relocation = code.atom_relocations().get(relocation_index).ok_or(
                BytecodeImageEncodeError::InvalidCodeSidecar {
                    function_index: function.zero_based(),
                    offset: u32::try_from(operand).unwrap_or(u32::MAX),
                },
            )?;
            if relocation.operand_offset() as usize != operand {
                return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
                    function_index: function.zero_based(),
                    offset: relocation.operand_offset(),
                });
            }
            relocation_index += 1;
        }
        expected_offset = end;
    }
    if expected_offset != code.as_bytes().len() || relocation_index != code.atom_relocations().len()
    {
        return Err(BytecodeImageEncodeError::InvalidCodeSidecar {
            function_index: function.zero_based(),
            offset: u32::try_from(expected_offset).unwrap_or(u32::MAX),
        });
    }
    Ok(())
}

fn key_atom(key: ImageKey) -> ImageAtom {
    match key {
        ImageKey::Index(index) => ImageAtom::Index(index),
        ImageKey::Predefined(atom) => ImageAtom::Predefined(atom),
        ImageKey::Dynamic(atom) => ImageAtom::Dynamic(atom),
    }
}

fn container_count(value: usize) -> Result<u32, BytecodeImageEncodeError> {
    u32::try_from(value).map_err(|_| {
        GraphError::CountOverflow {
            kind: GraphResourceKind::ContainerEntries,
        }
        .into()
    })
}

fn positive_u32(value: usize, field: FunctionField) -> Result<u32, FunctionEnvelopeError> {
    if value > MAX_QUICKJS_POSITIVE_INT {
        return Err(FunctionEnvelopeError::CountOverflow { field });
    }
    Ok(value as u32)
}

fn check_function_limit(
    limits: FunctionEnvelopeLimits,
    kind: FunctionResourceKind,
    requested: usize,
) -> Result<(), FunctionEnvelopeError> {
    let limit = limits.limit(kind);
    if requested > limit {
        return Err(FunctionEnvelopeError::ResourceLimit {
            kind,
            requested,
            limit,
        });
    }
    Ok(())
}

fn check_code_limit(
    limits: FunctionEnvelopeLimits,
    kind: CodeResourceKind,
    requested: usize,
) -> Result<(), FunctionEnvelopeError> {
    let limit = limits.code_limit(kind);
    if requested > limit {
        // The decoder knows byte length from the prefix, but discovers
        // instructions and atom relocations one at a time while scanning.
        // Preserve that observable requested count under stricter re-encode
        // policies instead of reporting the final sidecar length.
        let requested = match kind {
            CodeResourceKind::Bytes => requested,
            CodeResourceKind::Instructions | CodeResourceKind::AtomRelocations => {
                limit.checked_add(1).ok_or(FunctionEnvelopeError::Code(
                    CodeError::CountOverflow { kind },
                ))?
            }
        };
        return Err(FunctionEnvelopeError::Code(CodeError::ResourceLimit {
            kind,
            requested,
            limit,
        }));
    }
    Ok(())
}

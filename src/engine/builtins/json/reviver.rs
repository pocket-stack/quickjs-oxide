//! QuickJS-shaped `JSON.parse` builtin and reviver internalization.

use super::parse::JsonParseRecord;
use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::PropertyKeyKind;
use crate::engine::heap::ContextId;
use std::rc::Rc;

use crate::engine::object::operations::{InternalDefineResult, PropertyDefineOutcome};
use crate::engine::object::{
    CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::NativeArguments;

const MAX_JSON_REVIVER_DEPTH: usize = 128;

impl Runtime {
    pub(crate) fn call_json_parse(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(self, realm, ParseStep::start(self, realm, arguments)?)
    }

    fn define_json_reviver_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        let descriptor = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(true),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        Ok(
            match self.internal_define_own_property(realm, object, key, &descriptor)? {
                NativeConversion::Value(InternalDefineResult::Defined) => {
                    PropertyDefineOutcome::Defined(true)
                }
                NativeConversion::Value(
                    InternalDefineResult::RejectedOrdinary(_)
                    | InternalDefineResult::RejectedProxyTrap,
                ) => PropertyDefineOutcome::Defined(false),
                NativeConversion::Throw(value) => PropertyDefineOutcome::Throw(value),
            },
        )
    }
}

pub(crate) enum ParseStep {
    Complete(Completion),
    String { resume: ParseResume },
    Read { resume: ParseResume },
    Number { resume: ParseResume },
    Keys { resume: ParseResume },
    Enumerable { resume: ParseResume },
    Call { resume: ParseResume },
    Delete { resume: ParseResume },
    Define { resume: ParseResume },
}
pub(crate) struct ParseResume(Box<ParseResumeState>);
impl std::ops::Deref for ParseResume {
    type Target = ParseResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ParseResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ParseResume>() <= 8);
pub(crate) struct ParseResumeState {
    pending_effect: ParseStepPending,
    state: State,
    phase: Phase,
}
enum Phase {
    Source(Value),
    Read,
    Length,
    Number,
    Keys,
    Enumerable {
        keys: std::vec::IntoIter<PropertyKey>,
        selected: Vec<PropertyKey>,
        key: PropertyKey,
    },
    Revived,
    Applied,
}
pub(crate) struct State {
    realm: ContextId,
    source: JsString,
    reviver: Option<CallableRef>,
    frames: Vec<Node>,
    _record: Option<Rc<JsonParseRecord>>,
}
struct Node {
    holder: ObjectRef,
    key: PropertyKey,
    record: Option<Rc<JsonParseRecord>>,
    value: Value,
    context: Option<ObjectRef>,
    children: Children,
}
enum Children {
    None,
    Array { index: u32, length: u32 },
    Object(std::vec::IntoIter<PropertyKey>),
}
impl ParseStep {
    pub(crate) fn start(
        _runtime: &Runtime,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        Ok(Self::request_string(arguments.readable[0].clone(), {
            let phase = Phase::Source(arguments.readable[1].clone());
            let mut owner = Box::new(ParseResumeState {
                pending_effect: Default::default(),
                phase: Phase::Read,
                state: State {
                    realm,
                    source: JsString::from_static(""),
                    reviver: None,
                    frames: Vec::new(),
                    _record: None,
                },
            });
            owner.phase = phase;
            ParseResume(owner)
        }))
    }
}
impl ParseResumeState {
    fn top(&mut self) -> Result<&mut Node, RuntimeError> {
        self.frames
            .last_mut()
            .ok_or(RuntimeError::Invariant("JSON reviver lost its node"))
    }
    fn enter(
        mut self: Box<Self>,
        runtime: &Runtime,
        holder: ObjectRef,
        key: PropertyKey,
        record: Option<Rc<JsonParseRecord>>,
    ) -> Result<ParseStep, RuntimeError> {
        if self.frames.len() > MAX_JSON_REVIVER_DEPTH {
            return Ok(ParseStep::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(
                    self.realm,
                    NativeErrorKind::Internal,
                    "stack overflow",
                )?,
            )));
        }
        if self.frames.try_reserve(1).is_err() {
            return Ok(ParseStep::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(self.realm, NativeErrorKind::Internal, "out of memory")?,
            )));
        }
        self.frames.push(Node {
            holder: holder.clone(),
            key: key.clone(),
            record,
            value: Value::Undefined,
            context: None,
            children: Children::None,
        });
        Ok(ParseStep::request_read(holder, key, {
            let phase = Phase::Read;
            let mut owner = self;
            owner.phase = phase;
            ParseResume(owner)
        }))
    }
    fn next(mut self: Box<Self>, runtime: &Runtime) -> Result<ParseStep, RuntimeError> {
        let realm = self.realm;
        let node = self.top()?;
        let child = match &mut node.children {
            Children::None => None,
            Children::Array { index, length } if *index < *length => {
                let key = runtime.intern_property_key(&index.to_string())?;
                let record = node
                    .record
                    .as_mut()
                    .and_then(|record| record.array_child(*index as usize));
                *index += 1;
                Some((key, record))
            }
            Children::Array { .. } => None,
            Children::Object(keys) => keys.next().map(|key| {
                let record = node
                    .record
                    .as_mut()
                    .and_then(|record| record.object_child(&key));
                (key, record)
            }),
        };
        if let Some((key, record)) = child {
            let Value::Object(object) = &node.value else {
                return Err(RuntimeError::Invariant(
                    "JSON reviver child holder is not an object",
                ));
            };
            let object = object.clone();
            return self.enter(runtime, object, key, record);
        }
        let receiver = Value::Object(node.holder.clone());
        let name = Value::String(
            runtime
                .0
                .state
                .borrow()
                .atoms
                .to_js_string(node.key.atom())?,
        );
        let context = node
            .context
            .clone()
            .ok_or(RuntimeError::Invariant("JSON reviver lost its context"))?;
        let mut arguments = Vec::new();
        if arguments.try_reserve_exact(3).is_err() {
            return Ok(ParseStep::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Internal, "out of memory")?,
            )));
        }
        arguments.push(name);
        arguments.push(node.value.clone());
        arguments.push(Value::Object(context));
        let callable = self
            .reviver
            .clone()
            .ok_or(RuntimeError::Invariant("JSON reviver lost its callback"))?;
        Ok(ParseStep::request_call(callable, receiver, arguments, {
            let phase = Phase::Revived;
            let mut owner = self;
            owner.phase = phase;
            ParseResume(owner)
        }))
    }
    fn enumerate(
        mut self: Box<Self>,
        runtime: &Runtime,
        mut keys: std::vec::IntoIter<PropertyKey>,
        selected: Vec<PropertyKey>,
    ) -> Result<ParseStep, RuntimeError> {
        for key in keys.by_ref() {
            if runtime
                .0
                .state
                .borrow()
                .atoms
                .property_key_kind(key.atom())?
                != PropertyKeyKind::String
            {
                continue;
            }
            let Value::Object(object) = &self.top()?.value else {
                return Err(RuntimeError::Invariant(
                    "JSON reviver key holder is not an object",
                ));
            };
            return Ok(ParseStep::request_enumerable(
                object.clone(),
                key.clone(),
                {
                    let phase = Phase::Enumerable {
                        keys,
                        selected,
                        key,
                    };
                    let mut owner = self;
                    owner.phase = phase;
                    ParseResume(owner)
                },
            ));
        }
        self.top()?.children = Children::Object(selected.into_iter());
        self.next(runtime)
    }
}
impl ParseResume {
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<JsString>,
    ) -> Result<ParseStep, RuntimeError> {
        let source = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ParseStep::Complete(Completion::Throw(value)));
            }
        };
        let Phase::Source(reviver) = std::mem::replace(&mut self.0.phase, Phase::Read) else {
            return Err(RuntimeError::Invariant(
                "JSON parse unexpected string reply",
            ));
        };
        let mut state = self.0;
        state.source = source;
        state.reviver = match reviver {
            Value::Object(object) => runtime.as_callable(&object)?,
            _ => None,
        };
        // Holder allocation precedes parsing, as in the pinned implementation.
        let root = state
            .reviver
            .as_ref()
            .map(|_| runtime.new_ordinary_object_in_realm(state.realm))
            .transpose()?;
        let (parsed, record) =
            match runtime.parse_json_text(state.realm, &state.source, state.reviver.is_some())? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => {
                    return Ok(ParseStep::Complete(Completion::Throw(value)));
                }
            };
        let Some(root) = root else {
            return Ok(ParseStep::Complete(Completion::Return(parsed)));
        };
        let key = runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Literal0)?;
        match runtime.define_json_reviver_property(state.realm, &root, &key, parsed)? {
            PropertyDefineOutcome::Defined(true) => {}
            PropertyDefineOutcome::Defined(false) => {
                return Err(RuntimeError::Invariant(
                    "fresh JSON reviver root definition was rejected",
                ));
            }
            PropertyDefineOutcome::Throw(value) => {
                return Ok(ParseStep::Complete(Completion::Throw(value)));
            }
        }
        let record = record.map(Rc::new);
        // Keep the whole original parse graph alive until the root callback
        // returns, while child requests own stable immutable record handles.
        state._record = record.clone();
        state.enter(runtime, root, key, record)
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<ParseStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(ParseStep::Complete(Completion::Throw(value))),
        };
        match std::mem::replace(&mut self.0.phase, Phase::Read) {
            Phase::Read => {
                let realm = self.0.realm;
                let node = self.0.top()?;
                node.record = node.record.take().filter(|record| record.matches(&value));
                node.value = value.clone();
                node.context = Some(runtime.new_ordinary_object_in_realm(realm)?);
                if let Value::Object(object) = value {
                    let array =
                        match runtime.internal_is_array(realm, &Value::Object(object.clone()))? {
                            NativeConversion::Value(value) => value,
                            NativeConversion::Throw(value) => {
                                return Ok(ParseStep::Complete(Completion::Throw(value)));
                            }
                        };
                    if array {
                        Ok(ParseStep::request_read(
                            object,
                            runtime.pinned_property_key(
                                crate::engine::atom::pinned::PinnedAtom::Length,
                            )?,
                            {
                                let phase = Phase::Length;
                                let mut owner = self.0;
                                owner.phase = phase;
                                ParseResume(owner)
                            },
                        ))
                    } else {
                        Ok(ParseStep::request_keys(object, {
                            let phase = Phase::Keys;
                            let mut owner = self.0;
                            owner.phase = phase;
                            ParseResume(owner)
                        }))
                    }
                } else {
                    if let Some((start, end)) = node
                        .record
                        .as_ref()
                        .and_then(|record| record.primitive_span())
                    {
                        let context = node.context.clone().unwrap();
                        let source = Value::String(self.0.source.sub_string(start, end));
                        let key = runtime
                            .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Source)?;
                        match runtime.define_json_reviver_property(realm, &context, &key, source)? {
                            PropertyDefineOutcome::Defined(true) => {}
                            PropertyDefineOutcome::Defined(false) => {
                                return Err(RuntimeError::Invariant(
                                    "fresh JSON reviver source definition was rejected",
                                ));
                            }
                            PropertyDefineOutcome::Throw(value) => {
                                return Ok(ParseStep::Complete(Completion::Throw(value)));
                            }
                        }
                    }
                    self.0.next(runtime)
                }
            }
            Phase::Length => Ok(ParseStep::request_number(value, {
                let phase = Phase::Number;
                let mut owner = self.0;
                owner.phase = phase;
                ParseResume(owner)
            })),
            Phase::Revived => {
                let node = self
                    .0
                    .frames
                    .pop()
                    .ok_or(RuntimeError::Invariant("JSON reviver reply lost node"))?;
                if self.0.frames.is_empty() {
                    return Ok(ParseStep::Complete(Completion::Return(value)));
                }
                let resume = {
                    let phase = Phase::Applied;
                    let mut owner = self.0;
                    owner.phase = phase;
                    ParseResume(owner)
                };
                if matches!(value, Value::Undefined) {
                    Ok(ParseStep::request_delete(node.holder, node.key, resume))
                } else {
                    Ok(ParseStep::request_define(
                        node.holder,
                        node.key,
                        OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(value),
                            writable: DescriptorField::Present(true),
                            enumerable: DescriptorField::Present(true),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                        resume,
                    ))
                }
            }
            _ => Err(RuntimeError::Invariant(
                "JSON reviver unexpected value reply",
            )),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<f64>,
    ) -> Result<ParseStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "JSON reviver unexpected number reply",
            ));
        }
        let number = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ParseStep::Complete(Completion::Throw(value)));
            }
        };
        self.0.top()?.children = Children::Array {
            index: 0,
            length: Runtime::to_uint32_number(number),
        };
        self.0.next(runtime)
    }
    pub(crate) fn keys(
        self,
        runtime: &Runtime,
        reply: NativeConversion<Vec<PropertyKey>>,
    ) -> Result<ParseStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Keys) {
            return Err(RuntimeError::Invariant(
                "JSON reviver unexpected keys reply",
            ));
        }
        match reply {
            NativeConversion::Value(keys) => {
                self.0.enumerate(runtime, keys.into_iter(), Vec::new())
            }
            NativeConversion::Throw(value) => Ok(ParseStep::Complete(Completion::Throw(value))),
        }
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<bool>,
    ) -> Result<ParseStep, RuntimeError> {
        let value = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(ParseStep::Complete(Completion::Throw(value)));
            }
        };
        match std::mem::replace(&mut self.0.phase, Phase::Read) {
            Phase::Enumerable {
                keys,
                mut selected,
                key,
            } => {
                if value {
                    if selected.try_reserve(1).is_err() {
                        return Ok(ParseStep::Complete(Completion::Throw(
                            runtime.new_native_error_jsvalue(
                                self.0.realm,
                                NativeErrorKind::Internal,
                                "out of memory",
                            )?,
                        )));
                    }
                    selected.push(key);
                }
                self.0.enumerate(runtime, keys, selected)
            }
            Phase::Applied => self.0.next(runtime),
            _ => Err(RuntimeError::Invariant(
                "JSON reviver unexpected boolean reply",
            )),
        }
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ParseStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ParseStep::Complete(result) => return Ok(result),
            ParseStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            ParseStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            ParseStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            ParseStep::Keys { mut resume } => {
                let object = resume.take_keys_object();
                resume.keys(runtime, runtime.internal_own_property_keys(realm, &object)?)?
            }
            ParseStep::Enumerable { mut resume } => {
                let object = resume.take_enumerable_object();
                let key = resume.take_enumerable_key();
                resume.boolean(
                    runtime,
                    runtime.internal_snapshot_own_property_is_enumerable(realm, &object, &key)?,
                )?
            }
            ParseStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
            ParseStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                resume.boolean(
                    runtime,
                    runtime.internal_delete_property(realm, &object, &key)?,
                )?
            }
            ParseStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.boolean(
                    runtime,
                    match runtime.internal_define_own_property(realm, &object, &key, &descriptor)? {
                        NativeConversion::Value(result) => {
                            NativeConversion::Value(matches!(result, InternalDefineResult::Defined))
                        }
                        NativeConversion::Throw(value) => NativeConversion::Throw(value),
                    },
                )?
            }
        };
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    fn until_call(runtime: &Runtime, realm: ContextId, mut step: ParseStep) -> ParseStep {
        loop {
            step = match step {
                step @ ParseStep::Call { .. } => return step,
                ParseStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    resume
                        .resume(
                            runtime,
                            runtime.get_property_in_realm(realm, &object, &key).unwrap(),
                        )
                        .unwrap()
                }
                ParseStep::Keys { mut resume } => {
                    let object = resume.take_keys_object();
                    resume
                        .keys(
                            runtime,
                            runtime.internal_own_property_keys(realm, &object).unwrap(),
                        )
                        .unwrap()
                }
                ParseStep::Enumerable { mut resume } => {
                    let object = resume.take_enumerable_object();
                    let key = resume.take_enumerable_key();
                    resume
                        .boolean(
                            runtime,
                            runtime
                                .internal_snapshot_own_property_is_enumerable(realm, &object, &key)
                                .unwrap(),
                        )
                        .unwrap()
                }
                ParseStep::Delete { mut resume } => {
                    let object = resume.take_delete_object();
                    let key = resume.take_delete_key();
                    resume
                        .boolean(
                            runtime,
                            runtime
                                .internal_delete_property(realm, &object, &key)
                                .unwrap(),
                        )
                        .unwrap()
                }
                _ => panic!("unexpected reviver test step"),
            };
        }
    }
    #[test]
    fn parse_record_keeps_deleted_prior_children_until_callback_abandonment() {
        let runtime = Runtime::new();
        let weak = Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let callback = context.eval("(function(k,v){return v})").unwrap();
        let Value::Object(callback_object) = &callback else {
            panic!("expected callback");
        };
        let callback_id = callback_object.object_id();
        let arguments = NativeArguments {
            actual_arg_count: 2,
            readable: vec![
                Value::String(JsString::from_static("{\"a\":{},\"b\":{}}")),
                callback,
            ],
        };
        let ParseStep::String { mut resume } =
            ParseStep::start(&runtime, context.realm, &arguments).unwrap()
        else {
            panic!("expected source conversion");
        };
        let Value::String(source) = resume.take_string_value() else {
            panic!("expected payload");
        };
        let resident_owner = (&*resume.0) as *const ParseResumeState;
        drop(arguments);
        let step = until_call(
            &runtime,
            context.realm,
            resume
                .string(&runtime, NativeConversion::Value(source))
                .unwrap(),
        );
        let ParseStep::Call { mut resume } = step else {
            panic!("expected a callback");
        };
        let arguments = resume.take_call_arguments();
        let callable = resume.take_call_callable();
        let receiver = resume.take_call_receiver();
        assert_eq!(resident_owner, (&*resume.0) as *const ParseResumeState);
        drop(callable);
        drop(receiver);
        assert_eq!(arguments[0].to_js_string().unwrap().to_utf8_lossy(), "a");
        let Value::Object(first) = &arguments[1] else {
            panic!("expected first child");
        };
        let first_id = first.object_id();
        drop(arguments);
        let step = until_call(
            &runtime,
            context.realm,
            resume
                .resume(&runtime, Completion::Return(Value::Undefined))
                .unwrap(),
        );
        let ParseStep::Call { resume } = &step else {
            panic!("expected b callback");
        };
        let arguments = resume.0.pending_effect.call_arguments.as_ref().unwrap();
        assert_eq!(arguments[0].to_js_string().unwrap().to_utf8_lossy(), "b");
        let Value::Object(second) = &arguments[1] else {
            panic!("expected second child");
        };
        let second_id = second.object_id();
        let context_id = resume
            .state
            .frames
            .last()
            .unwrap()
            .context
            .as_ref()
            .unwrap()
            .object_id();
        let holder_id = resume.state.frames[0].holder.object_id();
        let Value::Object(root) = &resume.state.frames[0].value else {
            panic!("expected parsed root");
        };
        let root_id = root.object_id();
        let key = runtime.intern_property_key("a").unwrap();
        assert!(matches!(
            runtime
                .get_property_in_realm(context.realm, root, &key)
                .unwrap(),
            Completion::Return(Value::Undefined)
        ));
        drop(key);
        runtime.run_gc().unwrap();
        let ids = [
            first_id,
            second_id,
            root_id,
            holder_id,
            context_id,
            callback_id,
        ];
        for id in ids {
            assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        }
        drop(step);
        runtime.run_gc().unwrap();
        for id in ids {
            assert!(
                runtime.0.state.borrow().heap.object(id).is_err(),
                "abandoned reviver retained {id:?}"
            );
        }
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }
}

#[derive(Default)]
struct ParseStepPending {
    string_value: Option<Value>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    keys_object: Option<ObjectRef>,
    enumerable_object: Option<ObjectRef>,
    enumerable_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
}
impl ParseStep {
    pub(crate) fn request_string(value: Value, mut resume: ParseResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ParseResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: ParseResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_keys(object: ObjectRef, mut resume: ParseResume) -> Self {
        resume.0.pending_effect.keys_object = Some(object);
        Self::Keys { resume }
    }
    pub(crate) fn request_enumerable(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ParseResume,
    ) -> Self {
        resume.0.pending_effect.enumerable_object = Some(object);
        resume.0.pending_effect.enumerable_key = Some(key);
        Self::Enumerable { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: ParseResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ParseResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: ParseResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
}
impl ParseResume {
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("ParseStep String value")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ParseStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ParseStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("ParseStep Number value")
    }
    pub(crate) fn take_keys_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .keys_object
            .take()
            .expect("ParseStep Keys object")
    }
    pub(crate) fn take_enumerable_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .enumerable_object
            .take()
            .expect("ParseStep Enumerable object")
    }
    pub(crate) fn take_enumerable_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .enumerable_key
            .take()
            .expect("ParseStep Enumerable key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("ParseStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ParseStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ParseStep Call arguments")
    }
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("ParseStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("ParseStep Delete key")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("ParseStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("ParseStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("ParseStep Define descriptor")
    }
}
const _: () = assert!(std::mem::size_of::<ParseStep>() <= 64);

impl std::ops::Deref for ParseResumeState {
    type Target = State;
    fn deref(&self) -> &State {
        &self.state
    }
}
impl std::ops::DerefMut for ParseResumeState {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ParseStep>() <= 64);

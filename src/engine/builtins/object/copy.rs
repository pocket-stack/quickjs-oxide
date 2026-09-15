//! Object spread/rest share the pinned enumerable snapshot and live-read rules.
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    atom::PropertyKeyKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::Completion,
};
// These count this cursor's successful logical clone sites, not all runtime
// retains/releases and not bytes moved by the compiler.
#[inline]
fn clone_copy_object(value: &ObjectRef) -> ObjectRef {
    let copy = value.clone();
    #[cfg(all(feature = "profiling", feature = "stack-vm"))]
    crate::engine::api::profiling::record_owned_execution_event("copy_owner_clone.ObjectRef");
    copy
}
#[inline]
fn clone_copy_key(value: &PropertyKey) -> PropertyKey {
    let copy = value.clone();
    #[cfg(all(feature = "profiling", feature = "stack-vm"))]
    crate::engine::api::profiling::record_owned_execution_event("copy_owner_clone.PropertyKey");
    copy
}

pub(crate) enum CopyStep {
    Complete(Completion),
    #[cfg(feature = "stack-vm")]
    PreparedRead(Box<PreparedCopyRead>),
    Keys {
        object: ObjectRef,
        resume: CopyResume,
    },
    Enumerable {
        object: ObjectRef,
        key: PropertyKey,
        resume: CopyResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: CopyResume,
    },
}
#[cfg(feature = "stack-vm")]
pub(crate) struct PreparedCopyRead {
    pub(crate) read: crate::engine::object::OrdinaryRead,
    pub(crate) key: PropertyKey,
    pub(crate) resume: CopyResume,
}
pub(crate) struct CopyResume(Box<CopyResumeState>);
impl std::ops::Deref for CopyResume {
    type Target = CopyResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for CopyResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<CopyResume>() <= 8);
pub(crate) struct CopyResumeState {
    target: ObjectRef,
    source: ObjectRef,
    excluded: Option<ObjectRef>,
    snapshot: bool,
    remaining: std::vec::IntoIter<PropertyKey>,
    key: Option<PropertyKey>,
    rejection: &'static str,
}
impl CopyStep {
    pub(crate) fn start(
        runtime: &Runtime,
        target: ObjectRef,
        source: Value,
        excluded: Option<ObjectRef>,
    ) -> Result<Self, RuntimeError> {
        let Value::Object(source) = source else {
            if excluded.is_some() {
                return Err(RuntimeError::Invariant(
                    "object-rest source was not an Object after ToObject",
                ));
            }
            return Ok(Self::Complete(Completion::Return(Value::Undefined)));
        };
        if !target.belongs_to(runtime)
            || !source.belongs_to(runtime)
            || excluded
                .as_ref()
                .is_some_and(|value| !value.belongs_to(runtime))
        {
            return Err(RuntimeError::WrongRuntime("CopyDataProperties object"));
        }
        let snapshot = !runtime.is_proxy_object(&source)?;
        let rejection = if excluded.is_some() {
            "fresh Object rest result rejected a copied data property"
        } else {
            "fresh Object literal rejected a spread data property"
        };
        let resume = CopyResume(Box::new(CopyResumeState {
            target,
            source,
            excluded,
            snapshot,
            remaining: Vec::new().into_iter(),
            key: None,
            rejection,
        }));
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        crate::engine::api::profiling::record_owned_execution_event("copy_cursor_created");
        Ok(Self::Keys {
            object: clone_copy_object(&resume.source),
            resume,
        })
    }
    /// Initial validation selects a cursor without reading or defining any key.
    /// The VM consumes its source operand before advancing that cursor here.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn advance_without_callback(self, runtime: &Runtime) -> Result<Self, RuntimeError> {
        match self {
            Self::Keys { object: _, resume } if resume.snapshot => {
                #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                crate::engine::api::profiling::record_owned_execution_event("copy_local_own_keys");
                let keys = runtime.own_property_keys(&resume.source)?;
                resume.keys(runtime, NativeConversion::Value(keys))
            }
            selected => Ok(selected),
        }
    }
}
impl CopyResume {
    pub(crate) fn keys(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<Vec<PropertyKey>>,
    ) -> Result<CopyStep, RuntimeError> {
        let keys = match reply {
            NativeConversion::Value(keys) => keys,
            NativeConversion::Throw(value) => {
                return Ok(CopyStep::Complete(Completion::Throw(value)));
            }
        };
        let mut selected = Vec::new();
        for key in keys {
            let kind = runtime
                .0
                .state
                .borrow()
                .atoms
                .property_key_kind(key.atom())?;
            if !matches!(kind, PropertyKeyKind::String | PropertyKeyKind::Symbol) {
                continue;
            }
            // This optimization is only selected for non-Proxy sources. It
            // observes every descriptor before the first value getter runs.
            if self.0.snapshot {
                #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                crate::engine::api::profiling::record_owned_execution_event(
                    "copy_snapshot_descriptor_read",
                );
                if !runtime.own_property_is_enumerable(&self.0.source, &key)? {
                    continue;
                }
            }
            selected.push(key);
        }
        self.0.remaining = selected.into_iter();
        self.next(runtime)
    }
    fn next(mut self, runtime: &Runtime) -> Result<CopyStep, RuntimeError> {
        #[cfg(feature = "stack-vm")]
        let receiver = Value::Object(clone_copy_object(&self.0.source));
        while let Some(key) = self.0.remaining.next() {
            // Own membership never walks prototypes or calls getters.
            if let Some(excluded) = &self.0.excluded
                && runtime.has_own_property(excluded, &key)?
            {
                continue;
            }
            let key_copy = clone_copy_key(&key);
            #[cfg(all(feature = "profiling", feature = "stack-vm"))]
            if self.0.key.is_some() {
                crate::engine::api::profiling::record_owned_execution_event(
                    "copy_cursor_key_owner_replaced",
                );
            }
            self.0.key = Some(key_copy);
            #[cfg(feature = "stack-vm")]
            if self.0.snapshot {
                #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                crate::engine::api::profiling::record_owned_execution_event("copy_local_live_read");
                let read =
                    runtime.prepare_ordinary_read_borrowed(&self.0.source, &key, &receiver)?;
                match read {
                    crate::engine::object::OrdinaryRead::Complete(value) => {
                        self.define_value(runtime, value.unwrap_or(Value::Undefined))?;
                        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "object_copy_value_completed_locally",
                        );
                        continue;
                    }
                    read => {
                        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "copy_selected_read_publish",
                        );
                        return Ok(CopyStep::PreparedRead(Box::new(PreparedCopyRead {
                            read,
                            key,
                            resume: self,
                        })));
                    }
                }
            }
            return Ok(if self.0.snapshot {
                CopyStep::Read {
                    object: clone_copy_object(&self.0.source),
                    key,
                    resume: self,
                }
            } else {
                CopyStep::Enumerable {
                    object: clone_copy_object(&self.0.source),
                    key,
                    resume: self,
                }
            });
        }
        Ok(CopyStep::Complete(Completion::Return(Value::Undefined)))
    }

    fn define_value(&self, runtime: &Runtime, value: Value) -> Result<(), RuntimeError> {
        let key = self
            .0
            .key
            .as_ref()
            .ok_or(RuntimeError::Invariant("Object copy key missing"))?;
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        crate::engine::api::profiling::record_owned_execution_event("copy_define_attempt");
        runtime.define_fresh_object_descriptor_property(
            &self.0.target,
            key,
            value,
            self.0.rejection,
        )
    }
    pub(crate) fn boolean(
        self,
        runtime: &Runtime,
        reply: NativeConversion<bool>,
    ) -> Result<CopyStep, RuntimeError> {
        match reply {
            NativeConversion::Throw(value) => Ok(CopyStep::Complete(Completion::Throw(value))),
            NativeConversion::Value(false) => self.next(runtime),
            NativeConversion::Value(true) => Ok(CopyStep::Read {
                object: clone_copy_object(&self.0.source),
                key: clone_copy_key(
                    self.0
                        .key
                        .as_ref()
                        .ok_or(RuntimeError::Invariant("Object copy key missing"))?,
                ),
                resume: self,
            }),
        }
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<CopyStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(CopyStep::Complete(Completion::Throw(value))),
        };
        // Definition uses the unpublished target's own C_W_E data slot.
        self.define_value(runtime, value)?;
        self.next(runtime)
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: CopyStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            CopyStep::Complete(result) => return Ok(result),
            #[cfg(feature = "stack-vm")]
            CopyStep::PreparedRead(prepared) => {
                let PreparedCopyRead { read, key, resume } = *prepared;
                let completion = match runtime.finish_prepared_read(realm, &key, read)? {
                    NativeConversion::Value(value) => {
                        Completion::Return(value.unwrap_or(Value::Undefined))
                    }
                    NativeConversion::Throw(value) => Completion::Throw(value),
                };
                resume.resume(runtime, completion)?
            }
            CopyStep::Keys { object, resume } => {
                resume.keys(runtime, runtime.internal_own_property_keys(realm, &object)?)?
            }
            CopyStep::Enumerable {
                object,
                key,
                resume,
            } => resume.boolean(
                runtime,
                runtime.internal_own_property_is_enumerable(realm, &object, &key)?,
            )?,
            CopyStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
        };
    }
}

#[cfg(all(test, feature = "stack-vm"))]
mod recovery_tests {
    use super::*;

    #[test]
    fn recovery_copy_start_is_validation_only_and_local_advance_keeps_selected_getter() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let target = runtime.new_object(None).unwrap();
        let source = context
            .eval("globalThis.copyTrace=0;({a:1,get b(){copyTrace++;return 2}})")
            .unwrap();
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        let profile = crate::engine::api::profiling::CostProfile::start();
        let step = CopyStep::start(&runtime, target.clone(), source, None).unwrap();
        assert!(runtime.own_property_keys(&target).unwrap().is_empty());
        let step = step.advance_without_callback(&runtime).unwrap();
        assert!(matches!(step, CopyStep::PreparedRead(_)));
        assert_eq!(runtime.own_property_keys(&target).unwrap().len(), 1);
        assert_eq!(context.eval("copyTrace").unwrap(), Value::Int(0));
        assert!(matches!(
            finish(&runtime, context.realm, step).unwrap(),
            Completion::Return(Value::Undefined)
        ));
        assert_eq!(context.eval("copyTrace").unwrap(), Value::Int(1));
        assert_eq!(runtime.own_property_keys(&target).unwrap().len(), 2);
        #[cfg(all(feature = "profiling", feature = "stack-vm"))]
        {
            let costs = profile.snapshot();
            for (event, count) in [
                ("copy_local_own_keys", 1),
                ("copy_snapshot_descriptor_read", 2),
                ("copy_local_live_read", 2),
                ("copy_define_attempt", 2),
                ("copy_selected_read_publish", 1),
            ] {
                assert_eq!(
                    costs.owned_execution_events.get(event).copied(),
                    Some(count),
                    "{event}"
                );
            }
            assert_eq!(
                costs
                    .owned_execution_events
                    .get("copy_wait_handoff")
                    .copied()
                    .unwrap_or(0),
                0
            );
        }
    }

    #[test]
    fn recovery_copy_keeps_snapshot_live_reads_and_one_getter_execution() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context
            .eval(
                r#"(()=>{
            let calls=0, source={get a(){calls++;this.b=7;return 1},b:2};
            let copy={...source};
            return [calls,copy.a,copy.b,Object.keys(copy).join(',')].join('|');
        })()"#,
            )
            .unwrap();
        assert_eq!(
            result,
            Value::String(crate::engine::value::JsString::from_static("1|1|7|a,b"))
        );
    }

    #[test]
    fn recovery_copy_retains_proxy_order_and_partial_exception_identity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context.eval(r#"(()=>{
            let log=[], token={}, source=new Proxy({a:1,b:2},{
                ownKeys(o){log.push('keys');return ['a','b']},
                getOwnPropertyDescriptor(o,k){log.push('own:'+k);return Reflect.getOwnPropertyDescriptor(o,k)},
                get(o,k){log.push('get:'+k);if(k==='b')throw token;return o[k]}
            });
            let caught=false;try{({...source})}catch(e){caught=e===token}
            return [caught,log.join(',')].join('|');
        })()"#).unwrap();
        assert_eq!(
            result,
            Value::String(crate::engine::value::JsString::from_static(
                "true|keys,own:a,get:a,own:b,get:b"
            ))
        );
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<CopyStep>() <= 64);

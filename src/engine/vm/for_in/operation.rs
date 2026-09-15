//! For-in owns snapshot cursors and both prototype passes across Proxy callbacks.
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    atom::PropertyKeyKind,
    heap::{ContextId, ForInCandidate, ForInProperty},
    object::{ObjectRef, PropertyKey},
    value::{JsString, Value, conversion::NativeConversion},
};

pub(in crate::engine::vm) enum ForInStep {
    Complete {
        value: Value,
        done: Option<bool>,
    },
    Throw(Value),
    Keys {
        object: ObjectRef,
        resume: ForInResume,
    },
    Enumerable {
        object: ObjectRef,
        key: PropertyKey,
        resume: ForInResume,
    },
    Own {
        object: ObjectRef,
        key: PropertyKey,
        resume: ForInResume,
    },
    Prototype {
        object: ObjectRef,
        resume: ForInResume,
    },
}
pub(in crate::engine::vm) struct ForInResume(Box<ForInResumeState>);
impl std::ops::Deref for ForInResume {
    type Target = ForInResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ForInResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ForInResume>() <= 8);
pub(in crate::engine::vm) struct ForInResumeState {
    realm: ContextId,
    phase: Phase,
}
struct Probe {
    iterator: ObjectRef,
    base: ObjectRef,
    fast_array: bool,
}
enum AfterSnapshot {
    Start,
    Refresh { iterator: ObjectRef },
    Level { iterator: ObjectRef },
}
struct Snapshot {
    object: ObjectRef,
    keys: std::vec::IntoIter<PropertyKey>,
    properties: Vec<ForInProperty>,
    after: AfterSnapshot,
}
enum Phase {
    SnapshotKeys {
        object: ObjectRef,
        after: AfterSnapshot,
    },
    SnapshotEnumerable {
        snapshot: Snapshot,
        name: JsString,
    },
    ProbePrototype(Probe),
    ProbeKeys {
        probe: Probe,
        prototype: ObjectRef,
    },
    ProbeEnumerable {
        probe: Probe,
        prototype: ObjectRef,
        keys: std::vec::IntoIter<PropertyKey>,
    },
    LevelPrototype {
        iterator: ObjectRef,
    },
    Candidate {
        iterator: ObjectRef,
        name: JsString,
    },
}
impl ForInStep {
    /// Consume only the same non-Proxy branches used by query dispatch. These
    /// branches neither enter callbacks nor acquire continuation budget. Keep
    /// the exact selected Proxy step for its original budgeted dispatcher.
    pub(in crate::engine::vm) fn advance_without_callback(
        mut self,
        runtime: &Runtime,
    ) -> Result<Self, RuntimeError> {
        loop {
            self = match self {
                Self::Keys { object, resume } if !runtime.is_proxy_object(&object)? => {
                    let keys = runtime.own_property_keys(&object)?;
                    resume.keys(runtime, NativeConversion::Value(keys))?
                }
                Self::Enumerable {
                    object,
                    key,
                    resume,
                } if !runtime.is_proxy_object(&object)? => {
                    let reply = runtime.internal_snapshot_own_property_is_enumerable(
                        resume.realm,
                        &object,
                        &key,
                    )?;
                    resume.boolean(runtime, reply)?
                }
                Self::Own {
                    object,
                    key,
                    resume,
                } if !runtime.is_proxy_object(&object)? => {
                    let reply = runtime.internal_has_own_property(resume.realm, &object, &key)?;
                    resume.boolean(runtime, reply)?
                }
                Self::Prototype { object, resume } if !runtime.is_proxy_object(&object)? => {
                    let prototype = runtime.get_prototype_of(&object)?;
                    resume.prototype(runtime, NativeConversion::Value(prototype))?
                }
                step => return Ok(step),
            };
            #[cfg(all(feature = "profiling", feature = "stack-vm"))]
            crate::engine::api::profiling::record_owned_execution_event("for_in_local_step");
        }
    }

    pub(in crate::engine::vm) fn start(
        runtime: &Runtime,
        realm: ContextId,
        value: Value,
    ) -> Result<Self, RuntimeError> {
        let object = runtime.for_in_object(realm, value)?;
        let fast = object
            .as_ref()
            .map(|object| runtime.for_in_fast_array_count(object))
            .transpose()?
            .flatten();
        match object {
            Some(object) if fast.is_none() => snapshot(realm, object, AfterSnapshot::Start),
            object => Ok(ForInStep::Complete {
                value: Value::Object(runtime.allocate_for_in_iterator(
                    object.as_ref(),
                    fast,
                    Vec::new(),
                )?),
                done: None,
            }),
        }
    }

    pub(in crate::engine::vm) fn next(
        runtime: &Runtime,
        realm: ContextId,
        iterator: &ObjectRef,
    ) -> Result<Self, RuntimeError> {
        if !iterator.belongs_to(runtime) {
            return Err(RuntimeError::WrongRuntime("for-in iterator"));
        }
        advance(runtime, realm, iterator)
    }
}
fn snapshot(
    realm: ContextId,
    object: ObjectRef,
    after: AfterSnapshot,
) -> Result<ForInStep, RuntimeError> {
    Ok(ForInStep::Keys {
        object: object.clone(),
        resume: ForInResume(Box::new(ForInResumeState {
            realm,
            phase: Phase::SnapshotKeys { object, after },
        })),
    })
}
fn done() -> ForInStep {
    ForInStep::Complete {
        value: Value::Undefined,
        done: Some(true),
    }
}
fn advance(
    runtime: &Runtime,
    realm: ContextId,
    iterator: &ObjectRef,
) -> Result<ForInStep, RuntimeError> {
    loop {
        let candidate = runtime
            .0
            .state
            .borrow_mut()
            .heap
            .next_for_in_candidate(iterator.object_id())?;
        let (object, name, key) = match candidate {
            ForInCandidate::Done => return Ok(done()),
            ForInCandidate::BaseComplete { object, fast_array } => {
                let base = ObjectRef::from_borrowed_handle(runtime.clone(), object)?;
                return Ok(ForInStep::Prototype {
                    object: base.clone(),
                    resume: ForInResume(Box::new(ForInResumeState {
                        realm,
                        phase: Phase::ProbePrototype(Probe {
                            iterator: iterator.clone(),
                            base,
                            fast_array,
                        }),
                    })),
                });
            }
            ForInCandidate::LevelComplete(object) => {
                return Ok(ForInStep::Prototype {
                    object: ObjectRef::from_borrowed_handle(runtime.clone(), object)?,
                    resume: ForInResume(Box::new(ForInResumeState {
                        realm,
                        phase: Phase::LevelPrototype {
                            iterator: iterator.clone(),
                        },
                    })),
                });
            }
            ForInCandidate::ArrayIndex { object, index } => {
                let name = JsString::try_from_utf8(&index.to_string())?;
                // The hidden enumeration object retains its source. Recheck
                // the live dense prefix on every turn: deletion, shrinking,
                // or descriptor conversion must reach the ordinary fallback.
                // Presence needs neither a property atom nor a temporary owner.
                let dense_present = runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object(object)?
                    .dense_array_value(index)
                    .is_some();
                if dense_present {
                    record_local_step();
                    return Ok(ForInStep::Complete {
                        value: Value::String(name),
                        done: Some(false),
                    });
                }
                let key = if crate::engine::atom::Atom::from_immediate_integer(index).is_some() {
                    runtime.property_key_for_index(u64::from(index))?
                } else {
                    // Reuse the required JS key spelling for a non-immediate
                    // atom instead of formatting the same large index again.
                    runtime.intern_property_key_js_string(&name)?
                };
                (object, name, key)
            }
            ForInCandidate::Property { object, name } => {
                let key = runtime.intern_property_key_js_string(&name)?;
                (object, name, key)
            }
        };
        let object = ObjectRef::from_borrowed_handle(runtime.clone(), object)?;
        if runtime.is_proxy_object(&object)? {
            return Ok(ForInStep::Own {
                object,
                key,
                resume: ForInResume(Box::new(ForInResumeState {
                    realm,
                    phase: Phase::Candidate {
                        iterator: iterator.clone(),
                        name,
                    },
                })),
            });
        }
        // The iterator's resident snapshot owns the current object. No callback
        // can intervene here, so an ordinary candidate needs no resume owner.
        let reply = runtime.internal_has_own_property(realm, &object, &key)?;
        record_local_step();
        match reply {
            NativeConversion::Value(true) => {
                return Ok(ForInStep::Complete {
                    value: Value::String(name),
                    done: Some(false),
                });
            }
            NativeConversion::Value(false) => {}
            NativeConversion::Throw(value) => return Ok(ForInStep::Throw(value)),
        }
    }
}

fn record_local_step() {
    #[cfg(all(feature = "profiling", feature = "stack-vm"))]
    crate::engine::api::profiling::record_owned_execution_event("for_in_local_step");
}

impl ForInResume {
    pub(in crate::engine::vm) fn keys(
        self,
        runtime: &Runtime,
        reply: NativeConversion<Vec<PropertyKey>>,
    ) -> Result<ForInStep, RuntimeError> {
        let keys = match reply {
            NativeConversion::Value(keys) => keys,
            NativeConversion::Throw(value) => return Ok(ForInStep::Throw(value)),
        };
        match self.0.phase {
            Phase::SnapshotKeys { object, after } => snapshot_next(
                runtime,
                self.0.realm,
                Snapshot {
                    object,
                    after,
                    keys: keys.into_iter(),
                    properties: Vec::new(),
                },
            ),
            Phase::ProbeKeys { probe, prototype } => {
                probe_keys(runtime, self.0.realm, probe, prototype, keys.into_iter())
            }
            _ => Err(RuntimeError::Invariant("for-in keys reply has wrong phase")),
        }
    }
    pub(in crate::engine::vm) fn boolean(
        self,
        runtime: &Runtime,
        reply: NativeConversion<bool>,
    ) -> Result<ForInStep, RuntimeError> {
        let value = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(ForInStep::Throw(value)),
        };
        match self.0.phase {
            Phase::SnapshotEnumerable { mut snapshot, name } => {
                snapshot
                    .properties
                    .try_reserve(1)
                    .map_err(|_| RuntimeError::Invariant("for-in snapshot allocation failed"))?;
                snapshot.properties.push(ForInProperty {
                    name,
                    enumerable: value,
                });
                snapshot_next(runtime, self.0.realm, snapshot)
            }
            Phase::ProbeEnumerable {
                probe,
                prototype,
                keys,
            } => {
                if value {
                    enter_prototypes(runtime, self.0.realm, probe)
                } else {
                    probe_keys(runtime, self.0.realm, probe, prototype, keys)
                }
            }
            Phase::Candidate { iterator, name } => {
                if value {
                    Ok(ForInStep::Complete {
                        value: Value::String(name),
                        done: Some(false),
                    })
                } else {
                    advance(runtime, self.0.realm, &iterator)
                }
            }
            _ => Err(RuntimeError::Invariant(
                "for-in Boolean reply has wrong phase",
            )),
        }
    }
    pub(in crate::engine::vm) fn prototype(
        self,
        runtime: &Runtime,
        reply: NativeConversion<Option<ObjectRef>>,
    ) -> Result<ForInStep, RuntimeError> {
        let prototype = match reply {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(ForInStep::Throw(value)),
        };
        match self.0.phase {
            Phase::ProbePrototype(probe) => {
                if let Some(prototype) = prototype {
                    Ok(ForInStep::Keys {
                        object: prototype.clone(),
                        resume: Self(Box::new(ForInResumeState {
                            realm: self.0.realm,
                            phase: Phase::ProbeKeys { probe, prototype },
                        })),
                    })
                } else {
                    runtime.store_for_in_level(&probe.iterator, None, Vec::new())?;
                    Ok(done())
                }
            }
            Phase::LevelPrototype { iterator } => {
                if let Some(prototype) = prototype {
                    snapshot(self.0.realm, prototype, AfterSnapshot::Level { iterator })
                } else {
                    runtime.store_for_in_level(&iterator, None, Vec::new())?;
                    Ok(done())
                }
            }
            _ => Err(RuntimeError::Invariant(
                "for-in prototype reply has wrong phase",
            )),
        }
    }
}
fn snapshot_next(
    runtime: &Runtime,
    realm: ContextId,
    mut pending: Snapshot,
) -> Result<ForInStep, RuntimeError> {
    for key in pending.keys.by_ref() {
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
        let name = runtime.property_key_to_js_string(&key)?;
        if !runtime.is_proxy_object(&pending.object)? {
            let enumerable = match runtime.internal_snapshot_own_property_is_enumerable(
                realm,
                &pending.object,
                &key,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(ForInStep::Throw(value)),
            };
            pending
                .properties
                .try_reserve(1)
                .map_err(|_| RuntimeError::Invariant("for-in snapshot allocation failed"))?;
            pending.properties.push(ForInProperty { name, enumerable });
            record_local_step();
            continue;
        }
        return Ok(ForInStep::Enumerable {
            object: pending.object.clone(),
            key,
            resume: ForInResume(Box::new(ForInResumeState {
                realm,
                phase: Phase::SnapshotEnumerable {
                    snapshot: pending,
                    name,
                },
            })),
        });
    }
    match pending.after {
        AfterSnapshot::Start => Ok(ForInStep::Complete {
            value: Value::Object(runtime.allocate_for_in_iterator(
                Some(&pending.object),
                None,
                pending.properties,
            )?),
            done: None,
        }),
        AfterSnapshot::Refresh { iterator } => {
            runtime
                .0
                .state
                .borrow_mut()
                .heap
                .enter_for_in_prototype_chain(iterator.object_id(), Some(pending.properties))?;
            Ok(ForInStep::Prototype {
                object: pending.object,
                resume: ForInResume(Box::new(ForInResumeState {
                    realm,
                    phase: Phase::LevelPrototype { iterator },
                })),
            })
        }
        AfterSnapshot::Level { iterator } => {
            runtime.store_for_in_level(
                &iterator,
                Some(pending.object.object_id()),
                pending.properties,
            )?;
            advance(runtime, realm, &iterator)
        }
    }
}
fn probe_keys(
    runtime: &Runtime,
    realm: ContextId,
    probe: Probe,
    prototype: ObjectRef,
    mut keys: std::vec::IntoIter<PropertyKey>,
) -> Result<ForInStep, RuntimeError> {
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
        if !runtime.is_proxy_object(&prototype)? {
            let enumerable = match runtime
                .internal_snapshot_own_property_is_enumerable(realm, &prototype, &key)?
            {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(ForInStep::Throw(value)),
            };
            record_local_step();
            if enumerable {
                return enter_prototypes(runtime, realm, probe);
            }
            continue;
        }
        return Ok(ForInStep::Enumerable {
            object: prototype.clone(),
            key,
            resume: ForInResume(Box::new(ForInResumeState {
                realm,
                phase: Phase::ProbeEnumerable {
                    probe,
                    prototype,
                    keys,
                },
            })),
        });
    }
    Ok(ForInStep::Prototype {
        object: prototype,
        resume: ForInResume(Box::new(ForInResumeState {
            realm,
            phase: Phase::ProbePrototype(probe),
        })),
    })
}
fn enter_prototypes(
    runtime: &Runtime,
    realm: ContextId,
    probe: Probe,
) -> Result<ForInStep, RuntimeError> {
    if probe.fast_array {
        snapshot(
            realm,
            probe.base,
            AfterSnapshot::Refresh {
                iterator: probe.iterator,
            },
        )
    } else {
        runtime
            .0
            .state
            .borrow_mut()
            .heap
            .enter_for_in_prototype_chain(probe.iterator.object_id(), None)?;
        Ok(ForInStep::Prototype {
            object: probe.base,
            resume: ForInResume(Box::new(ForInResumeState {
                realm,
                phase: Phase::LevelPrototype {
                    iterator: probe.iterator,
                },
            })),
        })
    }
}
pub(in crate::engine::vm) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ForInStep,
) -> Result<(Value, Option<bool>), RuntimeError> {
    loop {
        step = match step {
            ForInStep::Complete { value, done } => return Ok((value, done)),
            ForInStep::Throw(value) => {
                runtime.set_pending_exception(value)?;
                return Err(RuntimeError::Exception);
            }
            ForInStep::Keys { object, resume } => {
                resume.keys(runtime, runtime.internal_own_property_keys(realm, &object)?)?
            }
            ForInStep::Enumerable {
                object,
                key,
                resume,
            } => resume.boolean(
                runtime,
                runtime.internal_snapshot_own_property_is_enumerable(realm, &object, &key)?,
            )?,
            ForInStep::Own {
                object,
                key,
                resume,
            } => resume.boolean(
                runtime,
                runtime.internal_has_own_property(realm, &object, &key)?,
            )?,
            ForInStep::Prototype { object, resume } => {
                resume.prototype(runtime, runtime.internal_get_prototype_of(realm, &object)?)?
            }
        };
    }
}

#[cfg(test)]
mod resident_tests {
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn dense_for_in_rechecks_descriptor_conversion_shrink_and_append() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let calls=0, names='';const a=[1,2,3];
            for(const k in a){
                names+=k;
                if(k==='0'){
                    Object.defineProperty(a,'1',{get(){calls++;throw 1;}});
                    delete a[2];a.push(4);
                }
            }
            if(names!=='01' || calls!==0)return false;
            names='';const b=[1,2,3];
            for(const k in b){names+=k;if(k==='0')b.length=1;}
            return names==='0';
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn resident_for_in_keeps_snapshot_shadowing_and_live_own_checks() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let gets=0;const proto={p:1,hidden:2},a=[3,4,5];
            Object.setPrototypeOf(a,proto);
            Object.defineProperty(a,'hidden',{value:3,enumerable:false});
            Object.defineProperty(a,'access',{get(){gets++;throw 1;},enumerable:true});
            let names='';for(const k in a){names+=k+',';if(k==='0')delete a[1];}
            if(names!=='0,2,access,p,' || gets!==0)return false;
            let n=0;const proxy=new Proxy({a:1},{ownKeys(){return ['a'];},getOwnPropertyDescriptor(){n++;return {value:1,writable:true,enumerable:n===1,configurable:true};},getPrototypeOf(){return null;}});
            names='';for(const k in proxy)names+=k;
            return names==='a' && n===2;
        })()"#).unwrap(), Value::Bool(true));
    }
}

#[cfg(all(test, feature = "stack-vm", feature = "profiling"))]
mod tests {
    use crate::engine::{
        api::{profiling::CostProfile, runtime::Runtime},
        value::Value,
        vm::Completion,
    };

    #[test]
    fn for_in_local_steps_keep_order_shadowing_deletion_and_accessor_silence() {
        for source in [
            "(function(){var calls=0,p={z:1,a:2},o=Object.create(p);o[2]=2;o[1]=1;Object.defineProperty(o,'a',{value:3,enumerable:false});Object.defineProperty(o,'b',{get(){calls++;throw 0},enumerable:true});o[Symbol('s')]=4;return function(){var names='';for(var k in o)names+=k+',';return names==='1,2,b,z,'&&calls===0?42:0}})()",
            "(function(){var o=[1,2,3];Object.setPrototypeOf(o,{p:4});return function(){var names='';for(var k in o){names+=k;if(k==='0')delete o[1]}return names==='02p'?42:0}})()",
            "(function(){var o=Object.create(null);o.a=1;o.b=2;return function(){var names='';for(var k in o){names+=k;if(k==='a'){delete o.b;o.c=3}}return names==='a'?42:0}})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let profile = CostProfile::start();
            let result = runtime
                .call_internal(context.realm, &callable, Value::Undefined, &[])
                .unwrap();
            let costs = profile.snapshot();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{source}: {result:?}"
            );
            assert!(
                costs
                    .owned_execution_events
                    .get("for_in_local_step")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "{source}: {costs:?}"
            );
            assert!(
                costs
                    .owned_execution_events
                    .get("for_in_completed_without_query")
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "{source}: {costs:?}"
            );
            assert_eq!(costs.legacy_dispatches, 0);
            assert_eq!(costs.owned_bridge_exits, 0);
            assert_eq!(costs.owned_sync_call_bridges, 0);
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }

    #[test]
    fn for_in_local_progress_leaves_proxy_admission_and_trap_untouched() {
        use super::ForInStep;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context.eval("globalThis.forInTrapCalls=0;new Proxy({a:1},{ownKeys(){forInTrapCalls++;throw 42}})").unwrap();
        let ForInStep::Keys { object, resume } =
            ForInStep::start(&runtime, context.realm, value).unwrap()
        else {
            panic!("expected selected Proxy ownKeys step");
        };
        let id = object.object_id();
        let step = ForInStep::Keys { object, resume }
            .advance_without_callback(&runtime)
            .unwrap();
        let ForInStep::Keys { object, .. } = step else {
            panic!("Proxy step must remain selected for budgeted dispatch");
        };
        assert_eq!(object.object_id(), id);
        assert!(matches!(
            context.eval("forInTrapCalls").unwrap(),
            Value::Int(0)
        ));
    }

    #[test]
    fn for_in_proxy_snapshots_and_double_prototype_probe_are_owned_without_replay() {
        for source in [
            "(function(){var baseProto=0,protoKeys=0;var proto=new Proxy({b:2},{ownKeys(t){protoKeys++;return Reflect.ownKeys(t)},getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)},getPrototypeOf(){return null}});var base=new Proxy({a:1},{ownKeys(t){return Reflect.ownKeys(t)},getOwnPropertyDescriptor(t,k){return Reflect.getOwnPropertyDescriptor(t,k)},getPrototypeOf(){baseProto++;return proto}});return function(){var names='';for(var key in base)names+=key;return names==='ab'&&baseProto===2&&protoKeys===2?42:0}})()",
            "(function(){var n=0;var base=new Proxy({a:1},{ownKeys(){return ['a']},getOwnPropertyDescriptor(){n++;return {value:1,writable:true,enumerable:n===1,configurable:true}},getPrototypeOf(){return null}});return function(){var names='';for(var key in base)names+=key;return names==='a'&&n===2?42:0}})()",
            "(function(){var marker={},calls=0,base=new Proxy({}, {ownKeys(){calls++;throw marker}});return function(){try{for(var key in base){}}catch(e){return calls===1&&e===marker?42:0}return 0}})()",
            "(function(){var marker={},calls=0,proto=new Proxy({p:2},{ownKeys(){calls++;throw marker}}),base=Object.create(proto);base.a=1;return function(){try{for(var key in base){}}catch(e){return calls===1&&e===marker?42:0}return 0}})()",
            "(function(){var base=[1,2],proto={p:3};Object.setPrototypeOf(base,proto);return function(){var names='';for(var key in base){names+=key;if(key==='0')delete base[1]}return names==='0p'?42:0}})()",
        ] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let callable = runtime
                .callable_from_value(context.eval(source).unwrap())
                .unwrap();
            let profile = CostProfile::start();
            let result = runtime
                .call_internal(context.realm, &callable, Value::Undefined, &[])
                .unwrap();
            let costs = profile.snapshot();
            assert!(
                matches!(result, Completion::Return(Value::Int(42))),
                "{source}: {result:?}"
            );
            assert_eq!(costs.legacy_dispatches, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_bridge_exits, 0, "{source}: {costs:?}");
            assert_eq!(costs.owned_sync_call_bridges, 0, "{source}: {costs:?}");
            assert!(runtime.0.state.borrow().active_frames.is_empty());
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ForInStep>() <= 64);

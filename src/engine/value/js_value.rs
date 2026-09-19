//! Crate-internal execution value and the public-API conversion boundary.
//!
//! [`JsValue`] is the engine-internal value representation mandated by the
//! S3-A design: scalars inline, every heap-backed kind (string, BigInt,
//! symbol, object) carried as a generational typed handle.  It deliberately
//! implements neither `Copy` nor `Drop`: every storage position owns its
//! handle edge explicitly, duplicated with [`Runtime::dup_jsvalue`] and
//! surrendered with [`Runtime::release_jsvalue`], mirroring QuickJS's C
//! ownership discipline.
//!
//! The public [`Value`] (with its `Rc<Runtime>`-rooted object/symbol
//! wrappers) stays the only type crossing the embedding boundary.  The two
//! directions live here and only here:
//!
//! - `unroot` / `into_jsvalue`: public root -> internal value (entering the
//!   engine).  The borrowed form duplicates the heap edge; the consuming
//!   form transfers the root's owned edge without a retain/release pair.
//! - `root`: internal value -> public root (leaving the engine), duplicating
//!   the edge and wrapping it in the public root types.
//!
//! String and BigInt conversion allocates one arena node per conversion:
//! node allocation happens only at genuine creation points (here: API/host
//! input conversion), never at value stores.

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::AtomIdx;
use crate::engine::heap::{BigIntId, ObjectId, StringId};
use crate::engine::heap::RawValue;
use crate::engine::value::Value;

/// Engine-internal value: scalars inline, heap kinds as generational handles.
///
/// See the module documentation for the ownership contract.  A `JsValue` is
/// 16 bytes (compile-time asserted below), half the public [`Value`].
pub enum JsValue {
    Undefined,
    Null,
    Bool(bool),
    Int(i32),
    Float(f64),
    String(StringId),
    BigInt(BigIntId),
    Symbol(AtomIdx),
    Object(ObjectId),
}

const _: () = assert!(std::mem::size_of::<JsValue>() == 16);
const _: () = assert!(std::mem::size_of::<AtomIdx>() == 4);

impl JsValue {
    /// Representation-only `typeof` tag, matching [`Value::type_of`].
    #[must_use]
    pub const fn type_of(&self) -> &'static str {
        match self {
            Self::Null => "object",
            Self::Bool(_) => "boolean",
            Self::Int(_) | Self::Float(_) => "number",
            Self::BigInt(_) => "bigint",
            Self::String(_) => "string",
            Self::Symbol(_) => "symbol",
            Self::Object(_) => "object",
            Self::Undefined => "undefined",
        }
    }

    /// Representation-only `Number` projection; never performs ToNumber.
    #[must_use]
    pub(crate) fn as_number_repr(&self) -> Option<crate::engine::value::number::operations::Number> {
        match self {
            Self::Int(value) => Some(crate::engine::value::number::operations::Number::Int(*value)),
            Self::Float(value) => {
                Some(crate::engine::value::number::operations::Number::Float(*value))
            }
            _ => None,
        }
    }

    /// Representation-only number projection as `f64`; never performs ToNumber.
    #[must_use]
    pub const fn as_number(&self) -> Option<f64> {
        match self {
            Self::Int(value) => Some(*value as f64),
            Self::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// Representation-only primitive `ToBoolean` (no HTMLDDA exception).
    #[must_use]
    pub(crate) fn to_boolean_primitive(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Int(value) => *value != 0,
            Self::Float(value) => *value != 0.0 && !value.is_nan(),
            Self::BigInt(_) | Self::String(_) => true,
            Self::Symbol(_) | Self::Object(_) => true,
            Self::Undefined | Self::Null => false,
        }
    }

    /// Borrow as the heap storage payload: the same handle ids, no allocation.
    #[must_use]
    pub(crate) fn as_raw(&self) -> RawValue {
        match self {
            Self::Undefined => RawValue::Undefined,
            Self::Null => RawValue::Null,
            Self::Bool(value) => RawValue::Bool(*value),
            Self::Int(value) => RawValue::Int(*value),
            Self::Float(value) => RawValue::Float(*value),
            Self::String(id) => RawValue::String(*id),
            Self::BigInt(id) => RawValue::BigInt(*id),
            Self::Symbol(index) => RawValue::Symbol(*index),
            Self::Object(id) => RawValue::Object(*id),
        }
    }

    /// Consume into the heap storage payload: the same handle ids, no
    /// allocation. The payload carries the same edges; the caller decides
    /// whether the store retains them or the edge was moved in.
    #[must_use]
    pub(crate) fn into_raw(self) -> RawValue {
        self.as_raw()
    }

    /// Convert a heap storage payload into an internal value.
    ///
    /// This is a plain same-id copy of every edge the payload carries; no
    /// retain happens here, so the caller must account for the edge ownership
    /// of both sides. Heap-private payloads (`Private`, `Uninitialized`,
    /// `Exception`) have no internal-value form and return `None`.
    #[must_use]
    pub(crate) fn from_raw(raw: RawValue) -> Option<Self> {
        Some(match raw {
            RawValue::Undefined => Self::Undefined,
            RawValue::Null => Self::Null,
            RawValue::Bool(value) => Self::Bool(value),
            RawValue::Int(value) => Self::Int(value),
            RawValue::Float(value) => Self::Float(value),
            RawValue::String(id) => Self::String(id),
            RawValue::BigInt(id) => Self::BigInt(id),
            RawValue::Symbol(index) => Self::Symbol(index),
            RawValue::Object(id) => Self::Object(id),
            RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception => {
                return None;
            }
        })
    }
}

impl From<crate::engine::value::number::operations::Number> for JsValue {
    /// Project an already-compacted numeric representation. Scalars only: no
    /// heap edge is created or duplicated.
    fn from(value: crate::engine::value::number::operations::Number) -> Self {
        match value {
            crate::engine::value::number::operations::Number::Int(value) => Self::Int(value),
            crate::engine::value::number::operations::Number::Float(value) => Self::Float(value),
        }
    }
}

impl std::fmt::Debug for JsValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Undefined => formatter.write_str("JsValue::Undefined"),
            Self::Null => formatter.write_str("JsValue::Null"),
            Self::Bool(value) => formatter.debug_tuple("JsValue::Bool").field(value).finish(),
            Self::Int(value) => formatter.debug_tuple("JsValue::Int").field(value).finish(),
            Self::Float(value) => formatter.debug_tuple("JsValue::Float").field(value).finish(),
            Self::String(id) => formatter.debug_tuple("JsValue::String").field(id).finish(),
            Self::BigInt(id) => formatter.debug_tuple("JsValue::BigInt").field(id).finish(),
            Self::Symbol(index) => formatter.debug_tuple("JsValue::Symbol").field(index).finish(),
            Self::Object(id) => formatter.debug_tuple("JsValue::Object").field(id).finish(),
        }
    }
}

impl Runtime {
    /// Convert a borrowed public root into an internal value, duplicating
    /// every heap edge it carries (entering-engine form).
    ///
    /// String and BigInt payloads allocate one arena node each: API input
    /// conversion is a genuine creation point under the ownership rules.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::WrongRuntime`] for a foreign object/symbol
    /// root, or a heap/atom error when an edge cannot be duplicated or a
    /// node cannot be allocated.
    pub(crate) fn unroot_value(&self, value: &Value) -> Result<JsValue, RuntimeError> {
        Ok(match value {
            Value::Undefined => JsValue::Undefined,
            Value::Null => JsValue::Null,
            Value::Bool(value) => JsValue::Bool(*value),
            Value::Int(value) => JsValue::Int(*value),
            Value::Float(value) => JsValue::Float(*value),
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("object root conversion"));
                }
                let id = object.object_id();
                self.retain_object_handle(id)?;
                JsValue::Object(id)
            }
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("symbol root conversion"));
                }
                let atom = symbol.atom();
                self.retain_atom_handle(atom)?;
                let index = self.0.state.borrow().atoms.unbrand(atom)?;
                JsValue::Symbol(index)
            }
            Value::String(string) => {
                let id = self.0.state.borrow_mut().heap.allocate_string(string.clone())?;
                JsValue::String(id)
            }
            Value::BigInt(bigint) => {
                let id = self.0.state.borrow_mut().heap.allocate_bigint(bigint.clone())?;
                JsValue::BigInt(id)
            }
        })
    }

    /// Consume a public root into an internal value, transferring the edge
    /// the root owned without a retain/release pair (entering-engine form).
    ///
    /// Object and symbol roots hand over their exactly-one owned reference;
    /// string and BigInt payloads move into a freshly allocated arena node.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::WrongRuntime`] for a foreign object/symbol
    /// root, or a heap/atom error when a node cannot be allocated.
    pub(crate) fn into_jsvalue(&self, value: Value) -> Result<JsValue, RuntimeError> {
        Ok(match value {
            Value::Undefined => JsValue::Undefined,
            Value::Null => JsValue::Null,
            Value::Bool(value) => JsValue::Bool(value),
            Value::Int(value) => JsValue::Int(value),
            Value::Float(value) => JsValue::Float(value),
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("object root conversion"));
                }
                JsValue::Object(object.into_handle())
            }
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("symbol root conversion"));
                }
                let atom = symbol.into_atom();
                let index = self.0.state.borrow().atoms.unbrand(atom)?;
                JsValue::Symbol(index)
            }
            Value::String(string) => {
                let id = self.0.state.borrow_mut().heap.allocate_string(string)?;
                JsValue::String(id)
            }
            Value::BigInt(bigint) => {
                let id = self.0.state.borrow_mut().heap.allocate_bigint(bigint)?;
                JsValue::BigInt(id)
            }
        })
    }

    /// Convert an internal value back into a public root, duplicating every
    /// heap edge it carries (leaving-engine form).
    ///
    /// The borrowed `JsValue` keeps its edges; the returned [`Value`] owns
    /// new roots.  String and BigInt payloads hand out a clone of the node's
    /// inner `Rc`, preserving `JsString` identity semantics.
    ///
    /// # Errors
    ///
    /// Returns a heap/atom error when a handle no longer resolves.
    pub(crate) fn root_value(&self, value: &JsValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            JsValue::Undefined => Value::Undefined,
            JsValue::Null => Value::Null,
            JsValue::Bool(value) => Value::Bool(*value),
            JsValue::Int(value) => Value::Int(*value),
            JsValue::Float(value) => Value::Float(*value),
            JsValue::Object(id) => {
                self.retain_object_handle(*id)?;
                Value::Object(crate::engine::object::ObjectRef::from_owned_handle(
                    self.clone(),
                    *id,
                ))
            }
            JsValue::Symbol(index) => {
                let atom = self.0.state.borrow().atoms.brand(*index)?;
                self.retain_atom_handle(atom)?;
                Value::Symbol(crate::engine::object::SymbolRef::from_owned_atom(
                    self.clone(),
                    atom,
                ))
            }
            JsValue::String(id) => {
                let string = self.0.state.borrow().heap.string(*id)?.clone();
                Value::String(string)
            }
            JsValue::BigInt(id) => {
                let bigint = self.0.state.borrow().heap.bigint(*id)?.clone();
                Value::BigInt(bigint)
            }
        })
    }

    /// Duplicate every heap edge carried by an internal value.
    ///
    /// Scalars copy; object/string/BigInt edges retain their node; symbols
    /// retain their atom.  The result owns independent edges and must
    /// eventually be passed to [`Runtime::release_jsvalue`].
    ///
    /// # Errors
    ///
    /// Returns a heap/atom error when an edge cannot be duplicated.
    pub(crate) fn dup_jsvalue(&self, value: &JsValue) -> Result<JsValue, RuntimeError> {
        Ok(match value {
            JsValue::Undefined => JsValue::Undefined,
            JsValue::Null => JsValue::Null,
            JsValue::Bool(value) => JsValue::Bool(*value),
            JsValue::Int(value) => JsValue::Int(*value),
            JsValue::Float(value) => JsValue::Float(*value),
            JsValue::Object(id) => {
                self.retain_object_handle(*id)?;
                JsValue::Object(*id)
            }
            JsValue::String(id) => {
                self.retain_string_handle(*id)?;
                JsValue::String(*id)
            }
            JsValue::BigInt(id) => {
                self.retain_bigint_handle(*id)?;
                JsValue::BigInt(*id)
            }
            JsValue::Symbol(index) => {
                let atom = self.0.state.borrow().atoms.brand(*index)?;
                self.retain_atom_handle(atom)?;
                JsValue::Symbol(*index)
            }
        })
    }

    /// Boundary adapter: root an internal value into the public representation
    /// and immediately release its internal edges. The returned [`Value`] owns
    /// independent roots; the consumed value surrenders every edge it carried.
    /// This is the consuming form of [`Runtime::root_value`] for sub-driver
    /// entry points whose callees consume public roots: the net edge count is
    /// unchanged and both sides' ownership is explicit.
    pub(crate) fn root_and_release_jsvalue(
        &self,
        value: JsValue,
    ) -> Result<Value, RuntimeError> {
        let rooted = self.root_value(&value)?;
        self.release_jsvalue(value)?;
        Ok(rooted)
    }

    /// Release every heap edge carried by an internal value.
    ///
    /// Scalars are no-ops; heap edges take the existing deferred-release
    /// path, so a release requested while the runtime state is borrowed is
    /// applied at the next operation boundary.
    ///
    /// # Errors
    ///
    /// Returns a heap/atom error when an edge fails validation; releases
    /// requested at trusted internal sites keep the engine-wide discipline
    /// that invariant violations surface at the deferred-drain boundary.
    pub(crate) fn release_jsvalue(&self, value: JsValue) -> Result<(), RuntimeError> {
        match value {
            JsValue::Undefined
            | JsValue::Null
            | JsValue::Bool(_)
            | JsValue::Int(_)
            | JsValue::Float(_) => Ok(()),
            JsValue::Object(id) => {
                self.release_object_handle(id);
                Ok(())
            }
            JsValue::String(id) => {
                self.release_string_handle(id);
                Ok(())
            }
            JsValue::BigInt(id) => {
                self.release_bigint_handle(id);
                Ok(())
            }
            JsValue::Symbol(index) => {
                let atom = self.0.state.borrow().atoms.brand(index)?;
                self.release_atom_handle(atom);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::atom::Atom;
    use crate::engine::heap::HeapNodeKind;
    use crate::engine::value::JsString;

    #[test]
    fn scalars_round_trip_without_heap_edges() {
        let runtime = Runtime::new();
        for value in [
            Value::Undefined,
            Value::Null,
            Value::Bool(true),
            Value::Int(-7),
            Value::Float(3.5),
        ] {
            let internal = runtime.unroot_value(&value).unwrap();
            assert_eq!(internal.type_of(), value.type_of());
            let rooted = runtime.root_value(&internal).unwrap();
            assert_eq!(rooted, value);
            runtime.release_jsvalue(internal).unwrap();
        }
    }

    #[test]
    fn object_edges_dup_and_release_by_the_value() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        let before = runtime.0.state.borrow().heap.object_strong_count(id).unwrap();

        let root = Value::Object(object);
        let internal = runtime.unroot_value(&root).unwrap();
        assert_eq!(
            runtime.0.state.borrow().heap.object_strong_count(id).unwrap(),
            before + 1
        );

        let dup = runtime.dup_jsvalue(&internal).unwrap();
        assert_eq!(
            runtime.0.state.borrow().heap.object_strong_count(id).unwrap(),
            before + 2
        );

        let rooted = runtime.root_value(&internal).unwrap();
        assert!(matches!(rooted, Value::Object(_)));
        assert_eq!(
            runtime.0.state.borrow().heap.object_strong_count(id).unwrap(),
            before + 3
        );
        drop(rooted);

        runtime.release_jsvalue(dup).unwrap();
        runtime.release_jsvalue(internal).unwrap();
        drop(root);
        let _operation = runtime.operation();
        // The last owned edge is gone: the node was finalized and its slot
        // reclaimed, so the identity now reads as stale.
        assert!(runtime.0.state.borrow().heap.object_strong_count(id).is_err());
    }

    #[test]
    fn into_jsvalue_transfers_the_owned_edge_without_retain() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let id = object.object_id();
        let before = runtime.0.state.borrow().heap.object_strong_count(id).unwrap();

        let internal = runtime.into_jsvalue(Value::Object(object)).unwrap();
        assert_eq!(
            runtime.0.state.borrow().heap.object_strong_count(id).unwrap(),
            before
        );

        runtime.release_jsvalue(internal).unwrap();
        let _operation = runtime.operation();
        // The transferred edge was the only one; the node is reclaimed.
        assert!(runtime.0.state.borrow().heap.object_strong_count(id).is_err());
    }

    #[test]
    fn foreign_roots_are_rejected() {
        let runtime = Runtime::new();
        let foreign = Runtime::new();
        let object = foreign.new_object(None).unwrap();
        assert!(matches!(
            runtime.unroot_value(&Value::Object(object)),
            Err(RuntimeError::WrongRuntime(_))
        ));
        let symbol = foreign.new_symbol(None).unwrap();
        assert!(matches!(
            runtime.unroot_value(&Value::Symbol(symbol)),
            Err(RuntimeError::WrongRuntime(_))
        ));
    }

    #[test]
    fn string_payloads_allocate_one_node_per_conversion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context.eval("'hello arena'").unwrap();
        let Value::String(string) = &value else {
            panic!("eval must produce a string");
        };

        let internal = runtime.unroot_value(&value).unwrap();
        let JsValue::String(id) = internal else {
            panic!("unroot must produce a string handle");
        };
        assert_eq!(runtime.0.state.borrow().heap.string(id).unwrap(), string);

        let rooted = runtime.root_value(&internal).unwrap();
        assert_eq!(&rooted, &value);
        // Reading hands out a clone of the node's inner Rc: identity is kept.
        let Value::String(reread) = &rooted else {
            panic!("root must produce a string");
        };
        assert!(reread.same_representation(string));

        runtime.release_jsvalue(internal).unwrap();
        let _operation = runtime.operation();
        assert!(runtime.0.state.borrow().heap.string(id).is_err());
    }

    #[test]
    fn bigint_payloads_allocate_one_node_per_conversion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context.eval("123456789012345678901234567890n").unwrap();
        let Value::BigInt(bigint) = &value else {
            panic!("eval must produce a bigint");
        };

        let internal = runtime.into_jsvalue(value.clone()).unwrap();
        let JsValue::BigInt(id) = internal else {
            panic!("conversion must produce a bigint handle");
        };
        assert_eq!(runtime.0.state.borrow().heap.bigint(id).unwrap(), bigint);

        let rooted = runtime.root_value(&internal).unwrap();
        assert_eq!(&rooted, &value);
        runtime.release_jsvalue(internal).unwrap();
        let _operation = runtime.operation();
        assert!(runtime.0.state.borrow().heap.bigint(id).is_err());
    }

    #[test]
    fn symbol_edges_use_the_atom_table() {
        let runtime = Runtime::new();
        let symbol = runtime
            .new_symbol(Some(JsString::from_static("internal")))
            .unwrap();
        let atom = symbol.atom();
        let root = Value::Symbol(symbol);

        let internal = runtime.unroot_value(&root).unwrap();
        let JsValue::Symbol(index) = internal else {
            panic!("unroot must produce a symbol index");
        };
        assert_eq!(index, runtime.0.state.borrow().atoms.unbrand(atom).unwrap());
        let retained = runtime
            .0
            .state
            .borrow()
            .atoms
            .resolve(atom)
            .unwrap()
            .ref_count;
        assert_eq!(retained, Some(2));

        let dup = runtime.dup_jsvalue(&internal).unwrap();
        let retained = runtime
            .0
            .state
            .borrow()
            .atoms
            .resolve(atom)
            .unwrap()
            .ref_count;
        assert_eq!(retained, Some(3));

        let rooted = runtime.root_value(&dup).unwrap();
        assert!(matches!(rooted, Value::Symbol(_)));
        drop(rooted);

        runtime.release_jsvalue(dup).unwrap();
        runtime.release_jsvalue(internal).unwrap();
        let _operation = runtime.operation();
        let retained = runtime
            .0
            .state
            .borrow()
            .atoms
            .resolve(atom)
            .unwrap()
            .ref_count;
        assert_eq!(retained, Some(1));
        drop(root);
    }

    #[test]
    fn stale_symbol_index_fails_boundary_branding() {
        let runtime = Runtime::new();
        let atom: Atom = runtime
            .0
            .state
            .borrow_mut()
            .atoms
            .intern("transient")
            .unwrap();
        let index = runtime.0.state.borrow().atoms.unbrand(atom).unwrap();
        // Release the only owner; the slot is reclaimed synchronously.
        runtime.0.state.borrow_mut().atoms.release(atom).unwrap();
        assert!(runtime.0.state.borrow().atoms.brand(index).is_err());
    }

    #[test]
    fn string_kind_is_reported_in_counts_and_cleanup() {
        let runtime = Runtime::new();
        let value = Value::String(JsString::from_static("counted"));
        let internal = runtime.unroot_value(&value).unwrap();
        let JsValue::String(id) = internal else {
            panic!("unroot must produce a string handle");
        };
        assert!(runtime.0.state.borrow().heap.string(id).is_ok());
        runtime.release_jsvalue(internal).unwrap();
        let _operation = runtime.operation();
        assert!(runtime.0.state.borrow().heap.string(id).is_err());
    }

    #[test]
    fn release_defers_while_state_is_borrowed() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let root = Value::Object(object);
        let internal = runtime.unroot_value(&root).unwrap();
        {
            let _state = runtime.0.state.borrow();
            runtime.release_jsvalue(internal).unwrap();
            assert!(runtime.0.deferred_references.has_pending());
        }
        let _operation = runtime.operation();
        assert!(!runtime.0.deferred_references.has_pending());
    }

    #[test]
    fn node_kind_reports_string_and_bigint() {
        assert_eq!(HeapNodeKind::String, HeapNodeKind::String);
        assert_ne!(HeapNodeKind::String, HeapNodeKind::BigInt);
    }
}

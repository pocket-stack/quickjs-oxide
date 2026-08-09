// Keep the Object oracle implementations in separate modules so their private
// helpers remain isolated while Cargo builds one integration target.

#[path = "oracle/object/oracle_object_accessors.rs"]
mod oracle_object_accessors;
#[path = "oracle/object/oracle_object_assign.rs"]
mod oracle_object_assign;
#[path = "oracle/object/oracle_object_assignment.rs"]
mod oracle_object_assignment;
#[path = "oracle/object/oracle_object_bindings.rs"]
mod oracle_object_bindings;
#[path = "oracle/object/oracle_object_descriptors.rs"]
mod oracle_object_descriptors;
#[path = "oracle/object/oracle_object_enumeration.rs"]
mod oracle_object_enumeration;
#[path = "oracle/object/oracle_object_extensibility.rs"]
mod oracle_object_extensibility;
#[path = "oracle/object/oracle_object_from_entries.rs"]
mod oracle_object_from_entries;
#[path = "oracle/object/oracle_object_group_by.rs"]
mod oracle_object_group_by;
#[path = "oracle/object/oracle_object_has_own.rs"]
mod oracle_object_has_own;
#[path = "oracle/object/oracle_object_integrity.rs"]
mod oracle_object_integrity;
#[path = "oracle/object/oracle_object_intrinsic.rs"]
mod oracle_object_intrinsic;
#[path = "oracle/object/oracle_object_is.rs"]
mod oracle_object_is;
#[path = "oracle/object/oracle_object_literals.rs"]
mod oracle_object_literals;
#[path = "oracle/object/oracle_object_methods.rs"]
mod oracle_object_methods;
#[path = "oracle/object/oracle_object_rest.rs"]
mod oracle_object_rest;
#[path = "oracle/object/oracle_object_super.rs"]
mod oracle_object_super;
#[path = "oracle/object/oracle_object_super_arrow.rs"]
mod oracle_object_super_arrow;
#[path = "oracle/object/oracle_object_super_eval.rs"]
mod oracle_object_super_eval;
#[path = "oracle/object/oracle_objects.rs"]
mod oracle_objects;

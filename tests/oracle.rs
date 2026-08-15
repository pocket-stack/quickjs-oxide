// Keep this registry sorted by wrapper path; the test262 aliases intentionally
// differ from their filenames so one filter selects the feature-gated host tests.
#[path = "support/object_graph_observation.rs"]
mod object_graph_observation;
#[path = "support/quickjs_argv_completion_oracle.rs"]
mod quickjs_argv_completion_oracle;
#[path = "support/quickjs_array_completion_oracle.rs"]
mod quickjs_array_completion_oracle;
#[path = "support/quickjs_oracle.rs"]
mod quickjs_oracle;
#[path = "support/quickjs_raw_source_oracle.rs"]
mod quickjs_raw_source_oracle;
#[path = "support/quickjs_syntax_diagnostic_oracle.rs"]
mod quickjs_syntax_diagnostic_oracle;
#[path = "support/runtime_completion_oracle.rs"]
mod runtime_completion_oracle;
#[path = "support/runtime_observation.rs"]
mod runtime_observation;
#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;
#[path = "support/mod.rs"]
mod support;

#[path = "oracle/raw_script_bytes.rs"]
mod raw_script_bytes;

#[path = "oracle_argument_semantics.rs"]
mod oracle_argument_semantics;
#[path = "oracle_array.rs"]
mod oracle_array;
#[path = "oracle_array_assignment.rs"]
mod oracle_array_assignment;
#[path = "oracle_array_methods.rs"]
mod oracle_array_methods;
#[path = "oracle_array_search.rs"]
mod oracle_array_search;
#[path = "oracle_array_unscopables.rs"]
mod oracle_array_unscopables;
#[path = "oracle_arrow_functions.rs"]
mod oracle_arrow_functions;
#[path = "oracle_async_functions.rs"]
mod oracle_async_functions;
#[path = "oracle_async_methods.rs"]
mod oracle_async_methods;
#[path = "oracle_atomics_non_shared.rs"]
mod oracle_atomics_non_shared;
#[path = "oracle_binary_data.rs"]
mod oracle_binary_data;
#[path = "oracle_class_base.rs"]
mod oracle_class_base;
#[path = "oracle_class_initialization.rs"]
mod oracle_class_initialization;
#[path = "oracle_collections.rs"]
mod oracle_collections;
#[path = "oracle_control_flow.rs"]
mod oracle_control_flow;
#[rustfmt::skip]
#[cfg(feature = "test262-host")]
#[path = "oracle_create_realm.rs"]
mod test262_create_realm;
#[path = "oracle_date_intrinsic.rs"]
mod oracle_date_intrinsic;
#[path = "oracle_error_semantics.rs"]
mod oracle_error_semantics;
#[path = "oracle_eval_semantics.rs"]
mod oracle_eval_semantics;
#[path = "oracle_exponentiation.rs"]
mod oracle_exponentiation;
#[path = "oracle_for_await_of.rs"]
mod oracle_for_await_of;
#[path = "oracle_function_apply.rs"]
mod oracle_function_apply;
#[path = "oracle_function_declarations.rs"]
mod oracle_function_declarations;
#[path = "oracle_function_semantics.rs"]
mod oracle_function_semantics;
#[path = "oracle_generator_yield_star_depth.rs"]
mod oracle_generator_yield_star_depth;
#[path = "oracle_global_semantics.rs"]
mod oracle_global_semantics;
#[rustfmt::skip]
#[cfg(feature = "test262-host")]
#[path = "oracle_host_gc.rs"]
mod test262_host_gc;
#[rustfmt::skip]
#[cfg(feature = "test262-host")]
#[path = "oracle_is_html_dda.rs"]
mod test262_is_html_dda;
#[path = "oracle_iterator_methods.rs"]
mod oracle_iterator_methods;
#[path = "oracle_json.rs"]
mod oracle_json;
#[path = "oracle_math_intrinsic.rs"]
mod oracle_math_intrinsic;
#[path = "oracle_member_access.rs"]
mod oracle_member_access;
#[path = "oracle_module_reentry.rs"]
mod oracle_module_reentry;
#[path = "oracle_number_kernels.rs"]
mod oracle_number_kernels;
#[path = "oracle_number_semantics.rs"]
mod oracle_number_semantics;
#[path = "oracle_object_semantics.rs"]
mod oracle_object_semantics;
#[path = "oracle_operator_semantics.rs"]
mod oracle_operator_semantics;
#[path = "oracle_parameters.rs"]
mod oracle_parameters;
#[path = "oracle_primitive_intrinsics.rs"]
mod oracle_primitive_intrinsics;
#[path = "oracle_primitives.rs"]
mod oracle_primitives;
#[path = "oracle_program_declarations.rs"]
mod oracle_program_declarations;
#[path = "oracle_promise.rs"]
mod oracle_promise;
#[path = "oracle_proxy_reflect.rs"]
mod oracle_proxy_reflect;
#[path = "oracle_regexp.rs"]
mod oracle_regexp;
#[path = "oracle_string_methods.rs"]
mod oracle_string_methods;
#[path = "oracle_template_semantics.rs"]
mod oracle_template_semantics;
#[path = "oracle_typed_array_methods.rs"]
mod oracle_typed_array_methods;
#[path = "oracle_unicode_lexical.rs"]
mod oracle_unicode_lexical;
#[path = "oracle_updates.rs"]
mod oracle_updates;
#[path = "oracle_vm_object_coercion.rs"]
mod oracle_vm_object_coercion;

// One oracle test executable; topic modules own their shared helpers.
#[path = "../common/mod.rs"]
mod common;
mod support;
use common::runtime as runtime_oracle;
use support::{
    object_graph_observation, quickjs_argv_completion_oracle, quickjs_array_completion_oracle,
    quickjs_oracle, quickjs_raw_source_oracle, quickjs_syntax_diagnostic_oracle,
    runtime_completion_oracle, runtime_observation,
};

mod raw_json_module_bytes;
mod raw_module_bytes;
mod raw_script_bytes;

mod arguments;
mod array;
mod array_assignment;
mod array_construction;
mod array_search;
mod array_unscopables;
mod arrow_functions;
mod async_functions;
mod async_methods;
mod atomics_non_shared;
mod binary_data;
mod class_base;
mod class_initialization;
mod collections;
mod control_flow;
mod date_intrinsic;
mod errors;
mod eval;
mod exponentiation;
mod for_await_of;
mod function_apply;
mod function_declarations;
mod function_semantics;
mod generator_yield_star_depth;
mod global;
mod iterator;
mod json;
mod math_intrinsic;
mod member_access;
mod module_reentry;
mod number;
mod number_kernels;
mod object;
mod operators;
mod parameters;
mod primitive_intrinsics;
mod primitives;
mod program_declarations;
mod promise;
mod proxy_reflect;
mod regexp;
mod string;
mod templates;
#[cfg(feature = "test262-host")]
mod test262_create_realm;
#[cfg(feature = "test262-host")]
mod test262_host_gc;
#[cfg(feature = "test262-host")]
mod test262_is_html_dda;
mod typed_array;
mod unicode_lexical;
mod update;
mod vm_object_coercion;

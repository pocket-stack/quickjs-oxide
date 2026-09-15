//! Profiling-only operation accounting for the resident Query dispatcher.
//!
//! One dispatch event describes the entry step, not every transition performed
//! inside a domain helper. Define stages are counted separately for that reason.

use super::request::{Resume, Step};
use crate::engine::api::profiling::record_owned_execution_event;

pub(super) fn record_dispatch(step: &Step) {
    record_owned_execution_event("query_dispatch");
    record_owned_execution_event(match step {
        Step::RootDescriptor { .. } => "query_dispatch.step.root_descriptor",
        Step::ModuleCallbackOperation { .. } => "query_dispatch.step.module_callback_operation",
        Step::ModuleBodyOperation { .. } => "query_dispatch.step.module_body_operation",
        Step::ModuleLink { .. } => "query_dispatch.step.module_link",
        Step::PromiseOperation { .. } => "query_dispatch.step.promise_operation",
        Step::IntrinsicPromiseResolve { .. } => "query_dispatch.step.intrinsic_promise_resolve",
        Step::ResumeFrame { .. } => "query_dispatch.step.resume_frame",
        Step::ForInComplete { .. } => "query_dispatch.step.for_in_complete",
        Step::TypedIteratorMethod { .. } => "query_dispatch.step.typed_iterator_method",
        Step::TypedIteratorMethodComplete { .. } => {
            "query_dispatch.step.typed_iterator_method_complete"
        }
        Step::TypedCollect { .. } => "query_dispatch.step.typed_collect",
        Step::TypedCollectComplete { .. } => "query_dispatch.step.typed_collect_complete",
        Step::TypedCreate { .. } => "query_dispatch.step.typed_create",
        Step::NumericComplete { .. } => "query_dispatch.step.numeric_complete",
        Step::NumericHtmlDda { .. } => "query_dispatch.step.numeric_html_dda",
        Step::TypedSpeciesView { .. } => "query_dispatch.step.typed_species_view",
        Step::RegExpSpecies { .. } => "query_dispatch.step.reg_exp_species",
        Step::RegExpSpeciesComplete { .. } => "query_dispatch.step.reg_exp_species_complete",
        Step::IndirectEval { .. } => "query_dispatch.step.indirect_eval",
        Step::Aggregate { .. } => "query_dispatch.step.aggregate",
        Step::OrdinaryPrimitive { .. } => "query_dispatch.step.ordinary_primitive",
        Step::ConstructorSource { .. } => "query_dispatch.step.constructor_source",
        Step::ConstructorSourceComplete { .. } => "query_dispatch.step.constructor_source_complete",
        Step::TypedSpecies { .. } => "query_dispatch.step.typed_species",
        Step::TypedSpeciesComplete { .. } => "query_dispatch.step.typed_species_complete",
        Step::ArrayCopy { .. } => "query_dispatch.step.array_copy",
        Step::OrdinaryInstance { .. } => "query_dispatch.step.ordinary_instance",
        Step::ParseIterator { .. } => "query_dispatch.step.parse_iterator",
        Step::String { .. } => "query_dispatch.step.string",
        Step::ObjectTag { .. } => "query_dispatch.step.object_tag",
        Step::RegExpExec { .. } => "query_dispatch.step.reg_exp_exec",
        Step::IteratorCloseWithResume { .. } => "query_dispatch.step.iterator_close_with_resume",
        Step::NativeRawComplete { .. } => "query_dispatch.step.native_raw_complete",
        Step::ArraySpecies { .. } => "query_dispatch.step.array_species",
        Step::ArrayPush { .. } => "query_dispatch.step.array_push",
        Step::IteratorNext { .. } => "query_dispatch.step.iterator_next",
        Step::IteratorNextComplete { .. } => "query_dispatch.step.iterator_next_complete",
        Step::IteratorCall { .. } => "query_dispatch.step.iterator_call",
        Step::IteratorClose { .. } => "query_dispatch.step.iterator_close",
        Step::Native { .. } => "query_dispatch.step.native",
        Step::Construct { .. } => "query_dispatch.step.construct",
        Step::ConstructProxy { .. } => "query_dispatch.step.construct_proxy",
        Step::ConstructorReady { .. } => "query_dispatch.step.constructor_ready",
        Step::Arguments { .. } => "query_dispatch.step.arguments",
        Step::ArgumentsComplete { .. } => "query_dispatch.step.arguments_complete",
        Step::SnapshotEnumerable { .. } => "query_dispatch.step.snapshot_enumerable",
        Step::OwnFlag { .. } => "query_dispatch.step.own_flag",
        Step::Keys { .. } => "query_dispatch.step.keys",
        Step::KeysComplete { .. } => "query_dispatch.step.keys_complete",
        Step::ReadValue { .. } => "query_dispatch.step.read_value",
        Step::PreparedHas { .. } => "query_dispatch.step.prepared_has",
        Step::PreparedRead { .. } => "query_dispatch.step.prepared_read",
        Step::Primitive { .. } => "query_dispatch.step.primitive",
        Step::GetPrototype { .. } => "query_dispatch.step.get_prototype",
        Step::SetPrototype { .. } => "query_dispatch.step.set_prototype",
        Step::Delete { .. } => "query_dispatch.step.delete",
        Step::PreventExtensions { .. } => "query_dispatch.step.prevent_extensions",
        Step::Element { .. } => "query_dispatch.step.element",
        Step::ElementComplete { .. } => "query_dispatch.step.element_complete",
        Step::TypedComplete { .. } => "query_dispatch.step.typed_complete",
        Step::Number { .. } => "query_dispatch.step.number",
        Step::NumberComplete { .. } => "query_dispatch.step.number_complete",
        Step::LengthComplete { .. } => "query_dispatch.step.length_complete",
        Step::SetLength { .. } => "query_dispatch.step.set_length",
        Step::SetComplete { .. } => "query_dispatch.step.set_complete",
        Step::PreparedSet { .. } => "query_dispatch.step.prepared_set",
        Step::SetContinue { .. } => "query_dispatch.step.set_continue",
        Step::SetSpecial { .. } => "query_dispatch.step.set_special",
        Step::Set { .. } => "query_dispatch.step.set",
        Step::SetProxy { .. } => "query_dispatch.step.set_proxy",
        Step::Define { .. } => "query_dispatch.step.define",
        Step::DefineOrdinary { .. } => "query_dispatch.step.define_ordinary",
        Step::Defined { .. } => "query_dispatch.step.defined",
        Step::Complete { .. } => "query_dispatch.step.complete",
        Step::BooleanComplete { .. } => "query_dispatch.step.boolean_complete",
        Step::OwnComplete { .. } => "query_dispatch.step.own_complete",
        Step::Converted { .. } => "query_dispatch.step.converted",
        Step::Has { .. } => "query_dispatch.step.has",
        Step::Read { .. } => "query_dispatch.step.read",
        Step::Call { .. } => "query_dispatch.step.call",
        Step::Descriptor { .. } => "query_dispatch.step.descriptor",
        Step::Extensible { .. } => "query_dispatch.step.extensible",
        Step::Convert { .. } => "query_dispatch.step.convert",
    });
    let resume = match step {
        Step::Read { resume, .. }
        | Step::PreparedRead { resume, .. }
        | Step::Set { resume, .. }
        | Step::PreparedSet { resume, .. }
        | Step::Define { resume, .. }
        | Step::DefineOrdinary { resume, .. }
        | Step::Primitive { resume, .. }
        | Step::Number { resume, .. }
        | Step::String { resume, .. }
        | Step::Call { resume, .. }
        | Step::Native { resume, .. }
        | Step::RegExpExec { resume, .. } => resume.as_ref(),
        _ => None,
    };
    // This is the immediate continuation owner, not a reconstructed call stack.
    // The residual bucket is intentional; every dispatch has exactly one owner
    // category and one step category, so both partitions can be reconciled.
    record_owned_execution_event(match resume {
        Some(Resume::RegExpExec(_)) => "query_dispatch.owner.regexp_exec",
        Some(Resume::RegExpReplace(_)) => "query_dispatch.owner.regexp_replace",
        Some(Resume::RegExpMatch(_)) => "query_dispatch.owner.regexp_match",
        Some(Resume::RegExpSearch(_)) => "query_dispatch.owner.regexp_search",
        Some(Resume::RegExpSplit(_)) => "query_dispatch.owner.regexp_split",
        Some(Resume::RegExpMatchAll(_)) => "query_dispatch.owner.regexp_match_all",
        Some(Resume::RegExpIterator(_)) => "query_dispatch.owner.regexp_iterator",
        Some(Resume::RegExpIteratorSet { .. }) => "query_dispatch.owner.regexp_iterator_set",
        Some(Resume::RegExpSpecies(_)) => "query_dispatch.owner.regexp_species",
        Some(Resume::RegExpConstructor(_)) => "query_dispatch.owner.regexp_constructor",
        Some(Resume::RegExpCompile(_)) => "query_dispatch.owner.regexp_compile",
        Some(Resume::RegExpPresentation(_)) => "query_dispatch.owner.regexp_presentation",
        Some(Resume::StringProtocol(_)) => "query_dispatch.owner.string_protocol",
        Some(_) => "query_dispatch.owner.other",
        None => "query_dispatch.owner.unclassified_step",
    });
}

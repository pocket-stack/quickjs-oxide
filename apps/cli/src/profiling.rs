//! CLI policy and reporting; the engine never formats or writes diagnostics.
use quickjs_oxide::engine::api::Runtime;

#[derive(Default)]
pub(crate) struct Options {
    pub dump: bool,
    pub trace: bool,
    pub json: bool,
    pub output: Option<String>,
    pub events: Option<usize>,
    pub iterations: Option<usize>,
}

impl Options {
    pub fn validate(&self, quit: bool) -> Result<(), &'static str> {
        if !cfg!(feature = "profiling")
            && (self.dump
                || self.trace
                || self.json
                || self.output.is_some()
                || self.events.is_some()
                || self.iterations.is_some())
        {
            return Err(
                "this build does not include profiling; rebuild quickjs-oxide-cli with --features profiling",
            );
        }
        if (self.json || self.output.is_some()) && !self.dump && !self.trace {
            return Err("--profile-json/--profile-output require -d or -T");
        }
        if self.events.is_some() && !self.trace {
            return Err("--profile-events requires -T");
        }
        if self.iterations.is_some() && !(quit && self.dump) {
            return Err("--profile-iterations requires -q -d");
        }
        Ok(())
    }
}

pub(crate) fn help() {
    println!("  -q, --quit        initialize and exit without evaluating a script");
    println!("  -d, --dump        memory and compile/VM diagnostics; with -q, lifecycle timing");
    println!("  -T, --trace       partial arena backing-storage allocation trace");
    println!("      --profile-json       emit versioned JSON Lines diagnostics");
    println!("      --profile-output PATH write diagnostics to a new file (default stderr)");
    println!("      --profile-events N   trace event limit, 0..1000000 (default 65536)");
    println!("      --profile-iterations N lifecycle samples, 1..10000 (default 100)");
    println!("  profiling support: {}", cfg!(feature = "profiling"));
}

#[cfg(not(feature = "profiling"))]
pub(crate) struct Session;
#[cfg(not(feature = "profiling"))]
impl Session {
    pub fn new(_: &Options) -> std::io::Result<Self> {
        Ok(Self)
    }
    pub fn runtime(&mut self) -> Runtime {
        Runtime::new_with_host_services(quickjs_oxide_host::SystemHostServices::default())
    }
    pub fn snapshot_guard<'a>(&'a self, _: &'a Runtime) -> SnapshotGuard {
        SnapshotGuard
    }
    pub fn lifecycle(&self) {}
}
#[cfg(not(feature = "profiling"))]
pub(crate) struct SnapshotGuard;
#[cfg(not(feature = "profiling"))]
impl SnapshotGuard {
    pub fn phase(&self, _: &'static str) {}
}

#[cfg(feature = "profiling")]
pub(crate) use enabled::*;

#[cfg(feature = "profiling")]
mod enabled {
    use super::*;
    use quickjs_oxide::engine::api::profiling::{
        AllocationEventKind, AllocationTrace, CostProfile, CostSnapshot, MemorySnapshot,
    };
    use std::cell::{Cell, RefCell};
    use std::io::{self, Write};
    use std::time::Instant;

    pub(crate) struct Session {
        writer: RefCell<Option<Box<dyn Write>>>,
        failed: Cell<bool>,
        dump: bool,
        trace_enabled: bool,
        json: bool,
        event_limit: usize,
        iterations: usize,
        trace: Option<AllocationTrace>,
        costs: Option<CostProfile>,
    }

    impl Session {
        pub fn new(options: &Options) -> io::Result<Self> {
            let writer: Option<Box<dyn Write>> = if options.dump || options.trace {
                Some(match &options.output {
                    Some(path) => Box::new(
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(path)?,
                    ),
                    None => Box::new(io::stderr()),
                })
            } else {
                None
            };
            Ok(Self {
                writer: RefCell::new(writer),
                failed: Cell::new(false),
                dump: options.dump,
                trace_enabled: options.trace,
                json: options.json,
                event_limit: options.events.unwrap_or(65_536),
                iterations: options.iterations.unwrap_or(100),
                trace: None,
                costs: options.dump.then(CostProfile::start),
            })
        }

        pub fn runtime(&mut self) -> Runtime {
            if self.trace_enabled {
                let (runtime, trace) = Runtime::new_with_allocation_trace(
                    quickjs_oxide_host::SystemHostServices::default(),
                    self.event_limit,
                );
                self.trace = Some(trace);
                runtime
            } else {
                Runtime::new_with_host_services(quickjs_oxide_host::SystemHostServices::default())
            }
        }

        fn write(&self, report: impl FnOnce(&mut dyn Write) -> io::Result<()>) {
            if self.failed.get() {
                return;
            }
            let result = {
                let mut output = self.writer.borrow_mut();
                let Some(writer) = output.as_mut() else {
                    return;
                };
                report(writer.as_mut()).and_then(|()| writer.flush())
            };
            if let Err(error) = result {
                self.failed.set(true);
                // Reporting errors must not replace the program's result/error.
                let _ = writeln!(io::stderr(), "qjs: profiling report incomplete: {error}");
            }
        }

        pub fn snapshot_guard<'a>(&'a self, runtime: &'a Runtime) -> SnapshotGuard<'a> {
            SnapshotGuard {
                session: self,
                runtime,
                phase: Cell::new("error-before-context-drop"),
            }
        }

        fn memory(&self, runtime: &Runtime, phase: &str) {
            if !self.dump {
                return;
            }
            let snapshot = runtime.memory_snapshot();
            self.write(|out| {
                if self.json { write_memory_json(out, &snapshot, phase) }
                else {
                    writeln!(out, "Oxide memory snapshot: runtime={} phase={} coverage=partial", snapshot.runtime_id, phase)?;
                    writeln!(out, "{:28} {:>12} {:>14} {:>16}", "CATEGORY", "COUNT", "INLINE USED B", "INLINE CAPACITY B")?;
                    for category in &snapshot.categories {
                        writeln!(out, "{:28} {:>12} {:>14} {:>16}", category.name, display_number(category.count), display_number(category.used_bytes), display_number(category.capacity_bytes))?;
                        writeln!(out, "  {}", category.basis)?;
                    }
                    writeln!(out, "heap={:?}; pending_jobs={}", snapshot.heap, snapshot.pending_jobs)?;
                    writeln!(out, "Total/allocator/peak/RSS bytes unavailable. Inline bytes exclude nested allocations; categories are not a total-memory estimate.")
                }
            });
        }

        pub fn lifecycle(&self) {
            if !self.dump {
                return;
            }
            // Profiling disabled in these independent runtimes. Host construction
            // occurs before the first timestamp, as it does for every sample.
            let mut samples = Vec::with_capacity(self.iterations);
            for _ in 0..self.iterations {
                let host = quickjs_oxide_host::SystemHostServices::default();
                let start = Instant::now();
                let runtime = Runtime::new_with_host_services(host);
                let created_runtime = Instant::now();
                let context = runtime.new_context();
                let created_context = Instant::now();
                drop(context);
                let dropped_context = Instant::now();
                drop(runtime);
                let end = Instant::now();
                samples.push([
                    (created_runtime - start).as_nanos(),
                    (created_context - created_runtime).as_nanos(),
                    (dropped_context - created_context).as_nanos(),
                    (end - dropped_context).as_nanos(),
                ]);
            }
            let minimum = std::array::from_fn::<_, 4, _>(|phase| {
                samples.iter().map(|s| s[phase]).min().unwrap()
            });
            self.write(|out| {
                if self.json {
                    write!(out, "{{\"schema\":\"oxide-lifecycle-v1\",\"metadata\":")?;
                    metadata(out)?;
                    write!(out, ",\"timer\":\"monotonic-wall\",\"unit\":\"ns\",\"instrumentation\":\"off\",\"iterations\":{},\"phases\":[\"runtime_create\",\"context_create\",\"context_drop\",\"runtime_drop\"],\"samples\":[", samples.len())?;
                    for (index, sample) in samples.iter().enumerate() {
                        if index != 0 { write!(out, ",")?; }
                        write!(out, "{sample:?}")?;
                    }
                    writeln!(out, "],\"minimum_per_phase_ns\":{:?},\"sum_of_phase_minima_ns\":{},\"aggregation\":\"independent-phase-minima; sum need not be one iteration\"}}", minimum, minimum.iter().sum::<u128>())
                } else {
                    writeln!(out, "Instantiation times: monotonic wall ns, {} samples; independent phase minima", samples.len())?;
                    writeln!(out, "Runtime create / Context create / Context drop / Runtime drop: {minimum:?}")?;
                    writeln!(out, "Sum of phase minima: {} ns (not necessarily one iteration)", minimum.iter().sum::<u128>())?;
                    for (index, sample) in samples.iter().enumerate() { writeln!(out, "sample {index}: {sample:?}")?; }
                    Ok(())
                }
            });
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            if let Some(costs) = &self.costs {
                let snapshot = costs.snapshot();
                if snapshot.parse.attempts != 0
                    || snapshot.legacy_dispatches != 0
                    || snapshot.owned_instructions != 0
                    || snapshot.owned_bridge_exits != 0
                    || snapshot.owned_sync_call_bridges != 0
                {
                    self.write(|out| write_costs(out, &snapshot, self.json));
                }
            }
            let Some(trace) = &self.trace else {
                return;
            };
            let snapshot = trace.snapshot();
            self.write(|out| {
                if self.json {
                    write!(out, "{{\"schema\":\"oxide-allocation-trace-v1\",\"metadata\":")?;
                    metadata(out)?;
                    write!(out, ",\"runtime_id\":{},\"coverage\":\"partial\",\"scope\":\"arena-slots-backing-storage\",\"basis\":\"safe-Vec-capacity-transitions; R does not imply libc realloc\",\"event_limit\":{},\"requested_event_limit\":{},\"buffer_allocation_failed\":{},\"dropped_events\":{},\"finished\":{},\"complete_within_scope\":{},\"unavailable\":[\"allocator-requested-bytes\",\"allocator-usable-bytes\",\"allocation-failures\",\"nested-payload-allocations\",\"timestamps\",\"call-stacks\"],\"events\":[", snapshot.runtime_id, snapshot.event_limit, snapshot.requested_event_limit, snapshot.buffer_allocation_failed, snapshot.dropped_events, snapshot.finished, snapshot.finished && snapshot.dropped_events == 0)?;
                    for (index, event) in snapshot.events.iter().enumerate() {
                        if index != 0 { write!(out, ",")?; }
                        write!(out, "{{\"sequence\":{},\"allocation_id\":{},\"kind\":\"{}\",\"old_capacity_bytes\":{},\"capacity_bytes\":{},\"requested_bytes\":null,\"usable_bytes\":null}}", event.sequence, event.allocation_id, event_kind(event.kind), event.old_capacity_bytes, event.capacity_bytes)?;
                    }
                    writeln!(out, "]}}")
                } else {
                    writeln!(out, "Oxide allocation trace: runtime={} coverage=partial scope=arena-slots-backing-storage; safe Vec capacity transitions, not a malloc interceptor", snapshot.runtime_id)?;
                    writeln!(out, "event_limit={} requested_event_limit={} buffer_allocation_failed={}", snapshot.event_limit, snapshot.requested_event_limit, snapshot.buffer_allocation_failed)?;
                    for event in &snapshot.events {
                        writeln!(out, "{} seq={} allocation={} capacity_bytes={} -> {}", event_kind(event.kind), event.sequence, event.allocation_id, event.old_capacity_bytes, event.capacity_bytes)?;
                    }
                    writeln!(out, "finished={} dropped_events={} complete_within_scope={}; requested/usable bytes and allocation failures unavailable", snapshot.finished, snapshot.dropped_events, snapshot.finished && snapshot.dropped_events == 0)
                }
            });
        }
    }

    pub(crate) struct SnapshotGuard<'a> {
        session: &'a Session,
        runtime: &'a Runtime,
        phase: Cell<&'static str>,
    }
    impl SnapshotGuard<'_> {
        pub fn phase(&self, phase: &'static str) {
            self.phase.set(phase);
        }
    }
    impl Drop for SnapshotGuard<'_> {
        fn drop(&mut self) {
            self.session.memory(self.runtime, self.phase.get());
        }
    }

    fn write_costs(out: &mut dyn Write, costs: &CostSnapshot, json: bool) -> io::Result<()> {
        if !json {
            writeln!(
                out,
                "Oxide compile/VM costs: scope=thread-interval, execution=see-owned-and-legacy-counters, timing=inclusive-and-exclusive-wall-ns (inclusive not additive); IR capacities=partial boundary snapshots"
            )?;
            writeln!(
                out,
                "parse={:?} resolution={:?} lowering={:?} blocks={:?} fusion={:?} relocation={:?} verify={:?} publish={:?}",
                costs.parse,
                costs.resolution,
                costs.lowering,
                costs.blocks,
                costs.fusion,
                costs.relocation,
                costs.verify,
                costs.publish
            )?;
            writeln!(
                out,
                "lowered_functions={} instructions={} code_inline_bytes={} maximum_verified_stack={}",
                costs.lowered_functions,
                costs.code_instructions,
                costs.code_inline_bytes,
                costs.maximum_verified_stack
            )?;
            writeln!(
                out,
                "owned_instructions={} bridge_exits={} sync_call_bridges={} max_operand_depth={}",
                costs.owned_instructions,
                costs.owned_bridge_exits,
                costs.owned_sync_call_bridges,
                costs.owned_max_operand_depth
            )?;
            writeln!(
                out,
                "owned_storage={:?} (partial; per-store peaks; logical transfers)",
                costs.owned_storage
            )?;
            writeln!(
                out,
                "call_preparation={:?} (successful preparation; cumulative capacities; partial root copies)",
                costs.call_preparation
            )?;
            writeln!(
                out,
                "call_buffers={:?} (producer-local cumulative counters; observed buffers are not allocations)",
                costs.call_buffers
            )?;
            for (name, phase) in &costs.vm_phases {
                writeln!(
                    out,
                    "vm_phase={name} attempts={} inclusive_ns={} exclusive_ns={} samples={} omitted={}",
                    phase.cost.attempts,
                    phase.cost.inclusive_ns,
                    phase.cost.exclusive_ns,
                    phase.samples_ns.len(),
                    phase.omitted_samples
                )?;
            }
            return writeln!(
                out,
                "legacy_dispatches={} pc_publications={} max_operand_depth={}; all-call allocations and total retain/release accounting unavailable",
                costs.legacy_dispatches,
                costs.legacy_pc_publications,
                costs.legacy_max_operand_depth
            );
        }
        write!(
            out,
            "{{\"schema\":\"oxide-compile-vm-cost-v1\",\"metadata\":"
        )?;
        metadata(out)?;
        write!(
            out,
            ",\"owned_storage\":{{\"coverage\":\"partial\",\"basis\":\"per-store-Vec-capacity-peaks-and-logical-owner-transfers; excludes bridge containers, cold payloads and primitive Rc events\""
        )?;
        for (name, value) in [
            (
                "slot_capacity_growths",
                costs.owned_storage.slot_capacity_growths as u64,
            ),
            (
                "maximum_slot_capacity",
                costs.owned_storage.maximum_slot_capacity as u64,
            ),
            (
                "frame_capacity_growths",
                costs.owned_storage.frame_capacity_growths as u64,
            ),
            (
                "maximum_frame_capacity",
                costs.owned_storage.maximum_frame_capacity as u64,
            ),
            ("frames_pushed", costs.owned_storage.frames_pushed as u64),
            (
                "maximum_frame_depth",
                costs.owned_storage.maximum_frame_depth as u64,
            ),
            (
                "slots_initialized",
                costs.owned_storage.slots_initialized as u64,
            ),
            (
                "physical_none_initializations",
                costs.owned_storage.physical_none_initializations,
            ),
            (
                "maximum_initialized_slots",
                costs.owned_storage.maximum_initialized_slots as u64,
            ),
            (
                "maximum_reserved_slots",
                costs.owned_storage.maximum_reserved_slots as u64,
            ),
            (
                "maximum_live_slots",
                costs.owned_storage.maximum_live_slots as u64,
            ),
            ("slot_moves", costs.owned_storage.slot_moves as u64),
            ("slot_clears", costs.owned_storage.slot_clears as u64),
            ("value_copies", costs.owned_storage.value_copies as u64),
            (
                "copied_heap_roots",
                costs.owned_storage.copied_heap_roots as u64,
            ),
            (
                "hot_value_releases",
                costs.owned_storage.hot_value_releases as u64,
            ),
            (
                "hot_heap_root_releases",
                costs.owned_storage.hot_heap_root_releases as u64,
            ),
        ] {
            write!(out, ",\"{}\":{}", name, value)?;
        }
        write!(out, "}}")?;
        write!(out, ",\"owned_execution_layouts\":{{")?;
        for (index, (name, [size, align])) in costs.owned_execution_layouts.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "\"{name}\":{{\"size_bytes\":{size},\"align_bytes\":{align}}}"
            )?;
        }
        write!(out, "}}")?;
        write!(out, ",\"owned_execution_events\":{{")?;
        for (index, (name, value)) in costs.owned_execution_events.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            write!(out, "\"{name}\":{value}")?;
        }
        write!(out, "}}")?;
        write!(
            out,
            ",\"call_buffers_scope\":\"producer-local-successful-capacity-and-copy-observations; not total allocator or retain/release accounting\""
        )?;
        write!(out, ",\"call_buffers\":{{")?;
        for (index, (name, cost)) in costs.call_buffers.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            write!(out, "\"{name}\":{{")?;
            for (field_index, (field, value)) in cost.fields().into_iter().enumerate() {
                if field_index != 0 {
                    write!(out, ",")?;
                }
                write!(out, "\"{field}\":{value}")?;
            }
            write!(out, "}}")?;
        }
        write!(out, "}}")?;
        write!(
            out,
            ",\"vm_phase_samples_basis\":\"first-4096-attempts-per-phase; includes errors; [inclusive,exclusive] ns; child phases share one exclusive clock\""
        )?;
        write!(out, ",\"vm_phases\":{{")?;
        for (index, (name, phase)) in costs.vm_phases.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "\"{name}\":{{\"attempts\":{},\"inclusive_ns\":{},\"exclusive_ns\":{},\"sample_limit\":4096,\"omitted_samples\":{},\"samples_ns\":[",
                phase.cost.attempts,
                phase.cost.inclusive_ns,
                phase.cost.exclusive_ns,
                phase.omitted_samples
            )?;
            for (sample_index, [inclusive, exclusive]) in phase.samples_ns.iter().enumerate() {
                if sample_index != 0 {
                    write!(out, ",")?;
                }
                write!(out, "[{inclusive},{exclusive}]")?;
            }
            write!(out, "]}}")?;
        }
        write!(out, "}}")?;
        write!(
            out,
            ",\"call_preparation\":{{\"coverage\":\"bytecode-preparation-and-owned-frame-storage\",\"basis\":\"cumulative-capacities; argument-buffers-are-observations; excludes bound/apply scratch and total Runtime retains\""
        )?;
        for (name, value) in [
            ("frames_prepared", costs.call_preparation.frames_prepared),
            (
                "parameter_buffer_allocations",
                costs.call_preparation.parameter_buffer_allocations,
            ),
            (
                "parameter_capacity_bytes",
                costs.call_preparation.parameter_capacity_bytes,
            ),
            (
                "local_buffer_allocations",
                costs.call_preparation.local_buffer_allocations,
            ),
            (
                "local_capacity_bytes",
                costs.call_preparation.local_capacity_bytes,
            ),
            (
                "parameter_slots_initialized",
                costs.call_preparation.parameter_slots_initialized,
            ),
            (
                "local_slots_initialized",
                costs.call_preparation.local_slots_initialized,
            ),
            (
                "parameter_value_copies",
                costs.call_preparation.parameter_value_copies,
            ),
            (
                "parameter_heap_root_copies",
                costs.call_preparation.parameter_heap_root_copies,
            ),
            (
                "callee_heap_root_copies",
                costs.call_preparation.callee_heap_root_copies,
            ),
            (
                "owned_frame_allocations",
                costs.call_preparation.owned_frame_allocations,
            ),
            (
                "owned_frame_bytes",
                costs.call_preparation.owned_frame_bytes,
            ),
            (
                "owned_captured_reuse_allocations",
                costs.call_preparation.owned_captured_reuse_allocations,
            ),
            (
                "owned_captured_reuse_capacity_bytes",
                costs.call_preparation.owned_captured_reuse_capacity_bytes,
            ),
            (
                "owned_argument_buffers_observed",
                costs.call_preparation.owned_argument_buffers_observed,
            ),
            (
                "owned_argument_capacity_bytes",
                costs.call_preparation.owned_argument_capacity_bytes,
            ),
        ] {
            write!(out, ",\"{}\":{}", name, value)?;
        }
        write!(out, "}}")?;

        write!(
            out,
            ",\"scope\":\"thread-interval-innermost-collector\",\"execution_path\":\"{}\",\"timer\":\"inclusive-monotonic-wall-ns\",\"phase_totals_additive\":false,\"phases\":{{",
            if costs.owned_instructions != 0
                || costs.owned_bridge_exits != 0
                || costs.owned_sync_call_bridges != 0
            {
                "owned-stack-with-legacy-bridge"
            } else {
                "legacy"
            }
        )?;
        for (index, (name, phase)) in [
            ("parse", costs.parse),
            ("resolution", costs.resolution),
            ("lowering", costs.lowering),
            ("blocks", costs.blocks),
            ("fusion", costs.fusion),
            ("relocation", costs.relocation),
            ("verify", costs.verify),
            ("publish", costs.publish),
        ]
        .into_iter()
        .enumerate()
        {
            if index != 0 {
                write!(out, ",")?;
            }
            write!(
                out,
                "\"{}\":{{\"attempts\":{},\"inclusive_ns\":{},\"exclusive_ns\":{},\"storage_samples\":{},\"maximum_observed_ir_capacity_bytes\":{}}}",
                name,
                phase.attempts,
                phase.inclusive_ns,
                phase.exclusive_ns,
                phase.storage_samples,
                phase.maximum_observed_ir_capacity_bytes
            )?;
        }
        writeln!(
            out,
            "}},\"lowered_functions\":{},\"code_instructions\":{},\"code_inline_bytes\":{},\"maximum_verified_stack\":{},\"legacy_dispatches\":{},\"legacy_pc_publications\":{},\"legacy_max_operand_depth\":{},\"owned_instructions\":{},\"owned_bridge_exits\":{},\"owned_sync_call_bridges\":{},\"owned_max_operand_depth\":{},\"code_bytes_basis\":\"typed-instruction-inline-storage-excludes-boxed-operands-and-metadata\",\"ir_capacity_basis\":\"phase-boundary-owned-Vec-buffers-excludes-payloads-source-hash-tables-worklists\",\"unavailable\":[\"compile-peak-memory\",\"all-call-allocations\",\"primitive-rc-reference-events\",\"all-retain-release\"]}}",
            costs.lowered_functions,
            costs.code_instructions,
            costs.code_inline_bytes,
            costs.maximum_verified_stack,
            costs.legacy_dispatches,
            costs.legacy_pc_publications,
            costs.legacy_max_operand_depth,
            costs.owned_instructions,
            costs.owned_bridge_exits,
            costs.owned_sync_call_bridges,
            costs.owned_max_operand_depth
        )
    }

    fn event_kind(kind: AllocationEventKind) -> &'static str {
        match kind {
            AllocationEventKind::Allocate => "A",
            AllocationEventKind::Reallocate => "R",
            AllocationEventKind::Free => "F",
        }
    }
    fn display_number(value: Option<usize>) -> String {
        value.map_or_else(|| "unavailable".into(), |v| v.to_string())
    }
    fn number(out: &mut dyn Write, value: Option<usize>) -> io::Result<()> {
        match value {
            Some(value) => write!(out, "{value}"),
            None => write!(out, "null"),
        }
    }
    fn string(out: &mut dyn Write, value: &str) -> io::Result<()> {
        write!(out, "\"")?;
        for c in value.chars() {
            match c {
                '"' => write!(out, "\\\"")?,
                '\\' => write!(out, "\\\\")?,
                c if c <= '\u{1f}' => write!(out, "\\u{:04x}", u32::from(c))?,
                c => write!(out, "{c}")?,
            }
        }
        write!(out, "\"")
    }
    fn metadata(out: &mut dyn Write) -> io::Result<()> {
        write!(out, "{{\"engine\":\"quickjs-oxide\",\"version\":")?;
        string(out, env!("CARGO_PKG_VERSION"))?;
        write!(out, ",\"quickjs_compat\":")?;
        string(out, quickjs_oxide::QUICKJS_COMPAT_VERSION)?;
        write!(out, ",\"commit\":")?;
        string(
            out,
            option_env!("QUICKJS_OXIDE_BUILD_COMMIT").unwrap_or("unavailable"),
        )?;
        write!(out, ",\"os\":")?;
        string(out, std::env::consts::OS)?;
        write!(out, ",\"architecture\":")?;
        string(out, std::env::consts::ARCH)?;
        write!(
            out,
            ",\"pointer_bits\":{},\"debug_assertions\":{},\"profiling_feature\":true}}",
            usize::BITS,
            cfg!(debug_assertions)
        )
    }
    fn write_memory_json(
        out: &mut dyn Write,
        snapshot: &MemorySnapshot,
        phase: &str,
    ) -> io::Result<()> {
        write!(out, "{{\"schema\":\"oxide-memory-v1\",\"metadata\":")?;
        metadata(out)?;
        write!(out, ",\"runtime_id\":{},\"phase\":", snapshot.runtime_id)?;
        string(out, phase)?;
        write!(
            out,
            ",\"coverage\":\"partial\",\"pending_jobs\":{},\"categories\":{{",
            snapshot.pending_jobs
        )?;
        for (index, category) in snapshot.categories.iter().enumerate() {
            if index != 0 {
                write!(out, ",")?;
            }
            string(out, category.name)?;
            write!(out, ":{{\"count\":")?;
            number(out, category.count)?;
            write!(out, ",\"used_bytes\":")?;
            number(out, category.used_bytes)?;
            write!(out, ",\"capacity_bytes\":")?;
            number(out, category.capacity_bytes)?;
            write!(out, ",\"basis\":")?;
            string(out, category.basis)?;
            write!(out, "}}")?;
        }
        let h = &snapshot.heap;
        writeln!(
            out,
            "}},\"heap_states\":{{\"initializing\":{},\"live\":{},\"zero_queued\":{},\"finalizing\":{},\"zombies\":{},\"vacant\":{},\"retired\":{}}},\"allocator\":{{\"requested_live_bytes\":null,\"usable_live_bytes\":null}},\"unavailable\":[\"total-memory-bytes\",\"allocator-live-bytes\",\"peak-bytes\",\"rss-bytes\",\"all-string-storage\",\"shared-backing-bytes\"]}}",
            h.initializing, h.live, h.zero_queued, h.finalizing, h.zombies, h.vacant, h.retired
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::rc::Rc;

        struct FailingWriter(Rc<Cell<usize>>);
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                self.0.set(self.0.get() + 1);
                Err(io::Error::other("injected diagnostic failure"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        #[test]
        fn profiling_output_failure_is_latched_without_retry_or_panic() {
            let calls = Rc::new(Cell::new(0));
            let session = Session::new(&Options::default()).unwrap();
            *session.writer.borrow_mut() = Some(Box::new(FailingWriter(calls.clone())));
            session.write(|writer| writeln!(writer, "record"));
            session.write(|writer| writeln!(writer, "record"));
            assert!(session.failed.get());
            assert_eq!(calls.get(), 1);
        }

        #[test]
        fn profiling_json_escapes_controls_as_json_not_rust_literals() {
            let mut bytes = Vec::new();
            string(&mut bytes, "a\n\0\u{1f}\\\"中").unwrap();
            assert_eq!(
                String::from_utf8(bytes).unwrap(),
                r#""a\u000a\u0000\u001f\\\"中""#
            );
        }
    }
}

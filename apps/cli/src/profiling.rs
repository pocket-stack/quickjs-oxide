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
    println!("  -d, --dump        memory snapshot; with -q, lifecycle timing");
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
        AllocationEventKind, AllocationTrace, MemorySnapshot,
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

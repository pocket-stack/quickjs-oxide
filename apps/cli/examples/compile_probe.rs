//! Public compile API timing: source I/O, Runtime/Context creation, execution,
//! and teardown are outside the measured interval.
use quickjs_oxide::engine::api::Runtime;
use quickjs_oxide_host::SystemHostServices;
use std::{env, fs, time::Instant};

fn main() {
    let path = env::args().nth(1).expect("source path");
    if path == "--version" {
        println!("oxide-compile-probe 1");
        return;
    }
    let source = fs::read_to_string(&path).expect("UTF-8 benchmark source");
    #[cfg(feature = "profiling")]
    let costs = quickjs_oxide::engine::api::profiling::CostProfile::start();
    let runtime = Runtime::new_with_host_services(SystemHostServices::default());
    let mut context = runtime.new_context();
    let started = Instant::now();
    let function = context
        .compile_with_filename(&source, &path)
        .expect("compile frozen script");
    let elapsed = started.elapsed().as_nanos();
    std::hint::black_box(&function);
    println!("compile_ns:{elapsed}");
    // An instrumented probe is deliberately inadmissible as formal timing:
    // replay.py requires empty stderr. Use this only for phase attribution.
    #[cfg(feature = "profiling")]
    {
        let snapshot = costs.snapshot();
        for (name, phase) in [
            ("parse", snapshot.parse),
            ("resolution", snapshot.resolution),
            ("lowering", snapshot.lowering),
            ("blocks", snapshot.blocks),
            ("fusion", snapshot.fusion),
            ("relocation", snapshot.relocation),
            ("verify", snapshot.verify),
            ("publish", snapshot.publish),
        ] {
            eprintln!(
                "{{\"phase\":\"{name}\",\"attempts\":{},\"inclusive_ns\":{},\"exclusive_ns\":{}}}",
                phase.attempts, phase.inclusive_ns, phase.exclusive_ns
            );
        }
    }
}

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qjs"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn profiling_inactive_keeps_normal_output() {
    let output = run(&["-e", "print(42)"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    assert!(output.stderr.is_empty());
}

#[cfg(not(feature = "profiling"))]
#[test]
fn profiling_flags_explain_missing_build_feature() {
    for args in [
        vec!["-d", "-e", "print(42)"],
        vec!["-Tq"],
        vec!["--profile-json", "-q"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("does not include profiling"));
    }
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_reports_preserve_exception_and_cover_teardown() {
    let output = run(&[
        "-dT",
        "--profile-json",
        "-e",
        "print(42); throw new Error('probe')",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"42\n");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("probe"));
    assert!(error.contains("\"phase\":\"error-before-context-drop\""));
    assert!(error.contains("\"finished\":true"));
    assert!(error.contains("\"complete_within_scope\":true"));
    assert!(error.contains("\"kind\":\"F\""));
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_options_validate_scope_and_bounds() {
    for args in [
        vec!["-q", "--profile-events", "1"],
        vec!["-d", "--profile-iterations", "2", "-e", "42"],
        vec!["-qd", "--profile-iterations", "0"],
        vec!["-qT", "--profile-events", "1000001"],
    ] {
        assert_eq!(run(&args).status.code(), Some(2));
    }
}

#[cfg(feature = "profiling")]
#[test]
fn profiling_lifecycle_and_overflow_are_labelled() {
    let output = run(&[
        "-qdT",
        "--profile-json",
        "--profile-iterations",
        "2",
        "--profile-events",
        "0",
    ]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let report = String::from_utf8(output.stderr).unwrap();
    assert_eq!(report.lines().count(), 3);
    assert!(report.contains("\"timer\":\"monotonic-wall\""));
    assert!(report.contains("\"iterations\":2"));
    assert!(report.contains("\"complete_within_scope\":false"));
    assert!(report.contains("\"events\":[]"));
}

#[cfg(feature = "profiling")]
#[test]
fn compiler_vm_cost_report_labels_the_execution_path_and_failures() {
    let output = run(&[
        "-d",
        "--profile-json",
        "-e",
        // InstanceOf and print use the owned driver, including the real output boundary.
        "[] instanceof Array; print((function(x){return x+1})(41))",
    ]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"42\n");
    let report = String::from_utf8(output.stderr).unwrap();
    let costs = report
        .lines()
        .find(|line| line.contains("oxide-compile-vm-cost-v1"))
        .unwrap();
    assert!(costs.contains("\"exclusive_ns\":"));
    assert!(costs.contains("\"verify\":{\"attempts\":1,"));
    assert!(costs.contains("\"publish\":{\"attempts\":1,"));
    if cfg!(feature = "stack-vm") {
        assert!(costs.contains("\"execution_path\":\"owned-stack-with-legacy-bridge\""));
        assert!(!costs.contains("\"owned_instructions\":0"));
        assert!(costs.contains("\"owned_bridge_exits\":0"));
        assert!(costs.contains("\"legacy_dispatches\":0"));
        assert!(costs.contains("\"owned_sync_call_bridges\":0,"));
        assert!(!costs.contains("\"frames_pushed\":0"));
        assert!(!costs.contains("\"slot_capacity_growths\":0"));
        assert!(costs.contains("\"Next\":{\"size_bytes\":"));
        assert!(costs.contains("\"query_completed_without_callback\":"));
        assert!(costs.contains("\"runtime_pc_publication\":"));
    } else {
        assert!(costs.contains("\"execution_path\":\"legacy\""));
        assert!(costs.contains("\"owned_instructions\":0"));
        assert!(costs.contains("\"owned_bridge_exits\":0"));
        assert!(!costs.contains("\"legacy_dispatches\":0"));
        assert!(costs.contains("\"owned_sync_call_bridges\":0,"));
        assert!(costs.contains("\"frames_pushed\":0"));
        assert!(costs.contains("\"slot_capacity_growths\":0"));
    }
    assert!(costs.contains("\"owned_storage\":{\"coverage\":\"partial\""));
    assert!(costs.contains("\"maximum_live_slots\":"));
    assert!(costs.contains("\"owned_sync_call_bridges\":"));
    assert!(costs.contains(
        "\"call_preparation\":{\"coverage\":\"bytecode-preparation-and-owned-frame-storage\""
    ));
    assert!(!costs.contains("\"frames_prepared\":0"));
    if cfg!(feature = "stack-vm") {
        assert!(costs.contains("\"parameter_value_copies\":0"));
        assert!(costs.contains("\"ordinary_scalar_argv_elided\":1"));
        assert!(costs.contains("\"ordinary_return_direct\":1"));
    } else {
        assert!(!costs.contains("\"parameter_value_copies\":0"));
    }
    assert!(costs.contains("\"lowered_functions\":2"));
    assert!(costs.contains("\"phase_totals_additive\":false"));
    let failed = run(&["-d", "--profile-json", "-e", "let = ;"]);
    assert!(!failed.status.success());
    let report = String::from_utf8(failed.stderr).unwrap();
    let costs = report
        .lines()
        .find(|line| line.contains("oxide-compile-vm-cost-v1"))
        .unwrap();
    assert!(costs.contains("\"parse\":{\"attempts\":1,"));
    assert!(costs.contains("\"lowered_functions\":0"));
}

#[cfg(feature = "profiling")]
#[test]
fn call_buffer_and_suspension_diagnostics_are_scoped_and_sampled() {
    let output = run(&[
        "-d",
        "--profile-json",
        "-e",
        "function* g(){yield 7;}var i=g();print(i.next().value);i.next();",
    ]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"7\n");
    let report = String::from_utf8(output.stderr).unwrap();
    let costs = report
        .lines()
        .find(|line| line.contains("oxide-compile-vm-cost-v1"))
        .unwrap();
    assert!(costs.contains("\"call_buffers_scope\":\"producer-local"));
    assert!(costs.contains("\"native.readable\":{\"capacity_growths\":"));
    assert!(costs.contains("\"freeze.encode\":{\"attempts\":"));
    assert!(costs.contains("\"thaw.decode\":{\"attempts\":"));
    assert!(costs.contains("\"sample_limit\":4096,\"omitted_samples\":0,\"samples_ns\":[["));
}

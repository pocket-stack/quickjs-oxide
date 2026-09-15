expect_full_rewrite_table < "$boundary_dir/canaries/stage3d_canaries.txt"

# The owned root handoff must finish before the active bytecode guard exits.
expect_full_rewrite_rejected owned-normal-completion-remapping \
    stage3d-throw-critical-route src/engine/vm/host_bridge.rs \
    'owned::execute(host, input, arguments).and_then(|exit| exit.finish(self.clone()));' \
    'owned::execute(host, input, arguments).map(|_| Completion::Return(Value::Undefined));'
expect_full_rewrite_rejected owned-normal-feature-gate-bypass \
    stage3d-throw-critical-route src/engine/vm/host_bridge.rs \
    $'#[cfg(feature = "stack-vm")]\n        let result =\n            owned::execute(host, input, arguments).and_then(|exit| exit.finish(self.clone()));' \
    $'#[cfg(any())]\n        let result =\n            owned::execute(host, input, arguments).and_then(|exit| exit.finish(self.clone()));'

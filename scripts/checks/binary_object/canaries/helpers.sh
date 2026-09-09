run_expect_rejected() {
    local label=$1
    local diagnostic=$2
    local relative=$3
    local canary=$4
    local case_root=$tmp_dir/$label
    local output=$case_root.output
    trap "$(printf 'rm -rf -- %q; rm -f -- %q' "$case_root" "$output")" EXIT

    mkdir -p "$case_root"
    cp -R "$fixture/." "$case_root"
    printf '\n%s\n' "$canary" >> "$case_root/$relative"
    if QUICKJS_OXIDE_BOUNDARY_SELF_TEST_TOKEN=$boundary_self_test_token \
        "$script_dir/checks/check-binary-object-boundary.sh" --scan-only "$case_root" \
        > "$output" 2>&1; then
        die "binary-object boundary canary escaped: $label"
    fi
    if [[ $(<"$output") != *"$diagnostic"* ]]; then
        echo "error: binary-object boundary canary failed for the wrong reason: $label" >&2
        cat "$output" >&2
        exit 1
    fi
}

run_expect_rewrite_rejected() {
    local label=$1
    local diagnostic=$2
    local relative=$3
    local before=$4
    local after=$5
    local case_root=$tmp_dir/$label
    local output=$case_root.output
    trap "$(printf 'rm -rf -- %q; rm -f -- %q' "$case_root" "$output")" EXIT

    mkdir -p "$case_root"
    cp -R "$fixture/." "$case_root"
    python3 "$boundary_dir/canaries/rewrite_fixture.py" "$case_root/$relative" "$before" "$after"
    if QUICKJS_OXIDE_BOUNDARY_SELF_TEST_TOKEN=$boundary_self_test_token \
        "$script_dir/checks/check-binary-object-boundary.sh" --scan-only "$case_root" \
        > "$output" 2>&1; then
        die "binary-object boundary rewrite canary escaped: $label"
    fi
    if [[ $(<"$output") != *"$diagnostic"* ]]; then
        echo "error: binary-object boundary rewrite canary failed for the wrong reason: $label" >&2
        cat "$output" >&2
        exit 1
    fi
}

run_expect_full_rewrite_rejected() {
    local label=$1
    local diagnostic=$2
    local relative=$3
    local before=$4
    local after=$5
    local before2=${6-}
    local after2=${7-}
    local added_relative=${8-}
    local added_source=${9-}
    local case_root=$tmp_dir/$label
    local output=$case_root.output
    trap "$(printf 'rm -rf -- %q; rm -f -- %q' "$case_root" "$output")" EXIT

    mkdir -p "$case_root"
    local tree
    for tree in src apps adapters conformance examples tests; do
        mkdir -p "$case_root/$tree"
        cp -R "$repository_root/$tree/." "$case_root/$tree"
    done
    mkdir -p "$case_root/apps/cli/tests/fixtures/inputs" "$case_root/apps/cli/tests/fixtures/expected" "$case_root/dev-support/test262/generated" "$case_root/docs"
    cp -- "$repository_root/Cargo.toml" "$case_root/Cargo.toml"
    cp -- "$repository_root/apps/cli/tests/fixtures/inputs/function_bytecode_wire.c" \
        "$case_root/apps/cli/tests/fixtures/inputs/function_bytecode_wire.c"
    cp -- "$repository_root/apps/cli/tests/fixtures/expected/function_bytecode_wire.quickjs-2026-06-04.txt" \
        "$case_root/apps/cli/tests/fixtures/expected/function_bytecode_wire.quickjs-2026-06-04.txt"
    cp -- "$repository_root/dev-support/quickjs-c-oracles.tsv" \
        "$case_root/dev-support/quickjs-c-oracles.tsv"
    cp -- "$repository_root/dev-support/test262/current.conf" \
        "$case_root/dev-support/test262/current.conf"
    cp -- "$repository_root/docs/status.md" "$case_root/docs/status.md"
    cp -- "$repository_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.tsv" \
        "$case_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.tsv"
    cp -- "$repository_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.jsonl" \
        "$case_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.jsonl"
    python3 "$boundary_dir/canaries/rewrite_source.py" "$case_root/$relative" "$before" "$after" "$before2" "$after2" \
        "$case_root" "$added_relative" "$added_source"
    if "$script_dir/checks/check-binary-object-boundary.sh" --scan-only "$case_root" \
        > "$output" 2>&1; then
        die "binary-object boundary full rewrite canary escaped: $label"
    fi
    if [[ $(<"$output") != *"$diagnostic"* ]]; then
        echo "error: binary-object boundary full rewrite canary failed for the wrong reason: $label" >&2
        cat "$output" >&2
        exit 1
    fi
}

expect_full_rewrite_table() {
    local label diagnostic relative before after
    while IFS='|' read -r label diagnostic relative before after; do
        [[ -n $label ]] || continue
        expect_full_rewrite_rejected "$label" "$diagnostic" "$relative" \
            "$(printf '%b' "$before")" "$(printf '%b' "$after")"
    done
}


expect_rejected() {
    queue_canary run_expect_rejected "$@"
}

expect_rewrite_rejected() {
    queue_canary run_expect_rewrite_rejected "$@"
}

expect_full_rewrite_rejected() {
    queue_canary run_expect_full_rewrite_rejected "$@"
}

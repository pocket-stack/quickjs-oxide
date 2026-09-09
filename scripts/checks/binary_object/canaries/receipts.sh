run_stage3i_receipt_escape_canaries() {
    local suite_root=$1
    local base_root=$suite_root/base

    mkdir -p "$base_root/apps/cli/tests/fixtures/inputs" "$base_root/apps/cli/tests/fixtures/expected" "$base_root/dev-support/test262/generated" \
        "$base_root/docs"
    local tree
    for tree in src apps adapters conformance examples tests; do
        mkdir -p "$base_root/$tree"
        cp -R "$repository_root/$tree/." "$base_root/$tree"
    done
    cp -- "$repository_root/Cargo.toml" "$base_root/Cargo.toml"
    cp -- "$repository_root/apps/cli/tests/fixtures/inputs/function_bytecode_wire.c" \
        "$base_root/apps/cli/tests/fixtures/inputs/function_bytecode_wire.c"
    cp -- "$repository_root/apps/cli/tests/fixtures/expected/function_bytecode_wire.quickjs-2026-06-04.txt" \
        "$base_root/apps/cli/tests/fixtures/expected/function_bytecode_wire.quickjs-2026-06-04.txt"
    cp -- "$repository_root/dev-support/quickjs-c-oracles.tsv" \
        "$base_root/dev-support/quickjs-c-oracles.tsv"
    cp -- "$repository_root/dev-support/test262/current.conf" \
        "$base_root/dev-support/test262/current.conf"
    cp -- "$repository_root/docs/status.md" "$base_root/docs/status.md"
    cp -- "$repository_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.tsv" \
        "$base_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.tsv"
    cp -- "$repository_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.jsonl" \
        "$base_root/dev-support/test262/generated/test262-class-private-callables-b-global-candidate.jsonl"

    expect_stage3i_receipt_multi_rewrite_rejected() {
        local label=$1
        local diagnostic=$2
        local plan=$3
        local field=${4-}
        local case_root=$suite_root/$label
        local output=$suite_root/$label.output

        cp -R "$base_root" "$case_root"
        python3 "$boundary_dir/canaries/rewrite_receipt.py" "$case_root" "$plan" "$field"
        if "$script_dir/checks/check-binary-object-boundary.sh" --scan-only "$case_root" \
            > "$output" 2>&1; then
            die "Stage3I receipt escape canary escaped: $label"
        fi
        if [[ $(<"$output") != *"$diagnostic"* ]]; then
            echo "error: Stage3I receipt escape canary failed for the wrong reason: $label" >&2
            cat "$output" >&2
            exit 1
        fi
    }

    local label field
    while IFS=: read -r label field; do
        expect_stage3i_receipt_multi_rewrite_rejected \
            "$label" stage3i-receipt-pin coherent-config-docs "$field"
    done < "$boundary_dir/canaries/stage3i_coherent_receipt_canaries.txt"
    expect_stage3i_receipt_multi_rewrite_rejected \
        stage3i-receipt-focused-content stage3i-focused-receipt focused-content
    expect_stage3i_receipt_multi_rewrite_rejected \
        stage3i-receipt-self-consistent-four-file-forgery stage3i-receipt-pin \
        self-consistent-four-file-forgery
    local wrapper
    for wrapper in \
        '<div style="display/**/:none">' \
        '<div style="display&#58;none">' \
        '<div style="visibility:collapse">'
    do
        label=${wrapper//[^A-Za-z0-9]/-}
        expect_stage3i_receipt_multi_rewrite_rejected \
            "stage3i-status-outside-wrapper-$label" stage3i-status \
            status-html-wrapper "$wrapper"
    done
    expect_stage3i_receipt_multi_rewrite_rejected \
        stage3i-receipt-true-hardlink stage3i-focused-receipt focused-hardlink
}

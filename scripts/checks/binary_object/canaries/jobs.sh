# Independent candidate trees can be scanned concurrently. Every result is waited
# for and checked; use QUICKJS_OXIDE_BOUNDARY_JOBS=1 for serial diagnostics.
canary_jobs=${QUICKJS_OXIDE_BOUNDARY_JOBS:-4}
[[ $canary_jobs =~ ^[1-9][0-9]*$ ]] || die 'QUICKJS_OXIDE_BOUNDARY_JOBS must be a positive integer'
canary_pids=()
canary_failures=0
canary_completed=0

wait_canary() {
    if ! wait "${canary_pids[0]}"; then
        canary_failures=$((canary_failures + 1))
    fi
    canary_pids=("${canary_pids[@]:1}")
    canary_completed=$((canary_completed + 1))
    if (( canary_completed % 50 == 0 )); then
        echo "binary-object canaries checked: $canary_completed" >&2
    fi
}

queue_canary() {
    "$@" &
    canary_pids+=("$!")
    if (( ${#canary_pids[@]} >= canary_jobs )); then
        wait_canary
    fi
}

finish_canaries() {
    while (( ${#canary_pids[@]} )); do wait_canary; done
    (( canary_failures == 0 )) || die "$canary_failures binary-object canaries failed"
    echo "binary-object canaries checked: $canary_completed; all rejected" >&2
}

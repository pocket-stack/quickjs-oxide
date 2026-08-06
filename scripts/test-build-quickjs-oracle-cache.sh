#!/usr/bin/env bash
set -euo pipefail

# Isolated regression coverage for build-quickjs-oracle.sh cache hardening.

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
build_script=$script_dir/build-quickjs-oracle.sh
version=2026-06-04
archive_name=quickjs-${version}.tar.xz
source_name=quickjs-${version}
fixture=${QJS_ORACLE_ARCHIVE_FIXTURE:-"$root/target/oracle/$archive_name"}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

if [[ ! -f "$fixture" || -L "$fixture" ]]; then
    fail "set QJS_ORACLE_ARCHIVE_FIXTURE to the verified $archive_name fixture"
fi

test_root=$(mktemp -d "${TMPDIR:-/tmp}/qjs-oracle-cache-test.XXXXXX")
trap 'rm -rf -- "$test_root"' EXIT HUP INT TERM
sequence=0

next_result_files() {
    sequence=$((sequence + 1))
    stdout_file=$test_root/stdout.$sequence
    stderr_file=$test_root/stderr.$sequence
}

assert_success_path() {
    local expected=$1
    shift
    local actual lines

    next_result_files
    if ! "$@" >"$stdout_file" 2>"$stderr_file"; then
        sed -n '1,160p' "$stderr_file" >&2
        fail "command unexpectedly failed: $*"
    fi
    lines=$(awk 'END { print NR }' "$stdout_file")
    actual=$(sed -n '1p' "$stdout_file")
    [[ "$lines" == 1 ]] || fail "stdout must contain exactly one line (got $lines): $*"
    [[ "$actual" == "$expected" ]] || fail "unexpected path '$actual' (expected '$expected')"
}

assert_failure() {
    local pattern=$1
    shift

    next_result_files
    if "$@" >"$stdout_file" 2>"$stderr_file"; then
        fail "command unexpectedly succeeded: $*"
    fi
    [[ ! -s "$stdout_file" ]] || fail "failed command wrote to stdout: $*"
    grep -F "$pattern" "$stderr_file" >/dev/null || {
        sed -n '1,160p' "$stderr_file" >&2
        fail "failure did not contain '$pattern': $*"
    }
}

new_cache() {
    local name=$1
    local cache=$test_root/$name

    mkdir -p -- "$cache"
    cp -- "$fixture" "$cache/$archive_name"
    printf '%s\n' "$cache"
}

prime_cache() {
    local name=$1
    local cache

    cache=$(new_cache "$name")
    assert_success_path "$cache/$source_name" \
        env QJS_ORACLE_CACHE="$cache" "$build_script" --source-only
    printf '%s\n' "$cache"
}

assert_no_internal_debris() {
    local cache=$1
    local debris

    debris=$(find "$cache" -maxdepth 1 \
        \( -name ".quickjs-${version}.work.*" \
           -o -name ".quickjs-${version}.archive.*" \
           -o -name ".quickjs-${version}.lock" \) -print)
    [[ -z "$debris" ]] || fail "cache contains internal debris: $debris"
    if [[ -d "$cache/$source_name" ]]; then
        debris=$(find "$cache/$source_name" -maxdepth 1 -name '.qjs.*.tmp' -print)
        [[ -z "$debris" ]] || fail "source contains publish debris: $debris"
    fi
}

fake_make=$test_root/fake-make-success.sh
cat >"$fake_make" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 3 && $1 == -C && $3 == qjs ]]
source_dir=$2
[[ ! -e "$source_dir/.obj/poison.d" ]]
if [[ -n ${FAKE_MAKE_LOG:-} ]]; then
    printf 'build %s\n' "$source_dir" >>"$FAKE_MAKE_LOG"
fi
sleep "${FAKE_MAKE_SLEEP:-0}"
printf '%s\n' '#!/usr/bin/env sh' '# built-from-clean-archive-stage' 'exit 0' >"$source_dir/qjs"
chmod +x "$source_dir/qjs"
EOF
chmod +x "$fake_make"

failing_make=$test_root/fake-make-failure.sh
cat >"$failing_make" <<'EOF'
#!/usr/bin/env bash
echo fake-make-failure >&2
exit 42
EOF
chmod +x "$failing_make"

copying_curl_dir=$test_root/copying-curl-bin
mkdir -- "$copying_curl_dir"
cat >"$copying_curl_dir/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=
while [[ $# -gt 0 ]]; do
    case $1 in
        -o|--output)
            output=$2
            shift 2
            ;;
        *) shift ;;
    esac
done
[[ -n "$output" ]]
cp -- "$QJS_TEST_FIXTURE" "$output"
EOF
chmod +x "$copying_curl_dir/curl"

bad_curl_dir=$test_root/bad-curl-bin
mkdir -- "$bad_curl_dir"
cat >"$bad_curl_dir/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=
while [[ $# -gt 0 ]]; do
    case $1 in
        -o|--output)
            output=$2
            shift 2
            ;;
        *) shift ;;
    esac
done
printf 'truncated archive\n' >"$output"
EOF
chmod +x "$bad_curl_dir/curl"

echo "[1/13] cold extraction, atomic archive download, and warm offline source-only" >&2
download_cache=$test_root/download
mkdir -- "$download_cache"
assert_success_path "$download_cache/$source_name" env \
    PATH="$copying_curl_dir:$PATH" QJS_TEST_FIXTURE="$fixture" \
    QJS_ORACLE_CACHE="$download_cache" "$build_script" --source-only
[[ -f "$download_cache/$archive_name" ]] || fail "verified download was not published"
assert_no_internal_debris "$download_cache"

offline_bin=$test_root/offline-bin
mkdir -- "$offline_bin"
offline_marker=$test_root/curl-was-called
cat >"$offline_bin/curl" <<EOF
#!/usr/bin/env bash
printf 'called\n' >'$offline_marker'
exit 99
EOF
chmod +x "$offline_bin/curl"
assert_success_path "$download_cache/$source_name" env \
    PATH="$offline_bin:$PATH" QJS_ORACLE_CACHE="$download_cache" \
    "$build_script" --source-only
[[ ! -e "$offline_marker" ]] || fail "warm source-only attempted network access"

echo "[2/13] cached archive checksum failure" >&2
bad_archive_cache=$(new_cache bad-archive)
printf 'corruption\n' >>"$bad_archive_cache/$archive_name"
assert_failure "archive checksum mismatch" env \
    QJS_ORACLE_CACHE="$bad_archive_cache" "$build_script" --source-only
[[ ! -e "$bad_archive_cache/$source_name" ]] || fail "bad archive published source"
assert_no_internal_debris "$bad_archive_cache"

echo "[3/13] failed download is never published" >&2
bad_download_cache=$test_root/bad-download
mkdir -- "$bad_download_cache"
assert_failure "archive checksum mismatch" env PATH="$bad_curl_dir:$PATH" \
    QJS_ORACLE_CACHE="$bad_download_cache" "$build_script" --source-only
[[ ! -e "$bad_download_cache/$archive_name" ]] || fail "bad download was published"
assert_no_internal_debris "$bad_download_cache"

echo "[4/13] changed archive-provided source member" >&2
changed_cache=$(prime_cache changed-source)
printf 'tampered\n' >>"$changed_cache/$source_name/VERSION"
assert_failure "source differs from verified archive" env \
    QJS_ORACLE_CACHE="$changed_cache" "$build_script" --source-only
assert_no_internal_debris "$changed_cache"

echo "[5/13] deleted archive-provided source member" >&2
deleted_cache=$(prime_cache deleted-source)
rm -- "$deleted_cache/$source_name/qjs.c"
assert_failure "source member has wrong type" env \
    QJS_ORACLE_CACHE="$deleted_cache" "$build_script" --source-only
assert_no_internal_debris "$deleted_cache"

echo "[6/13] symlink substituted for archive-provided source member" >&2
symlink_cache=$(prime_cache symlink-source)
rm -- "$symlink_cache/$source_name/qjs.c"
ln -s VERSION "$symlink_cache/$source_name/qjs.c"
assert_failure "source member has wrong type" env \
    QJS_ORACLE_CACHE="$symlink_cache" "$build_script" --source-only
assert_no_internal_debris "$symlink_cache"

echo "[7/13] partial persistent source is rejected" >&2
partial_cache=$(new_cache partial-source)
mkdir -- "$partial_cache/$source_name"
cp -- "$download_cache/$source_name/VERSION" "$partial_cache/$source_name/VERSION"
assert_failure "source member has wrong type" env \
    QJS_ORACLE_CACHE="$partial_cache" "$build_script" --source-only
assert_no_internal_debris "$partial_cache"

echo "[8/13] fake qjs and dependency files are ignored by a clean warm rebuild" >&2
poison_cache=$(prime_cache poisoned-build)
mkdir -- "$poison_cache/$source_name/.obj"
printf 'include attacker-controlled dependency\n' >"$poison_cache/$source_name/.obj/poison.d"
printf '%s\n' '#!/usr/bin/env sh' 'echo poisoned' >"$poison_cache/$source_name/qjs"
chmod +x "$poison_cache/$source_name/qjs"
make_log=$test_root/make.log
assert_success_path "$poison_cache/$source_name/qjs" env \
    FAKE_MAKE_LOG="$make_log" MAKE="$fake_make" \
    QJS_ORACLE_CACHE="$poison_cache" "$build_script"
grep -F '# built-from-clean-archive-stage' "$poison_cache/$source_name/qjs" >/dev/null || \
    fail "poisoned qjs was reused"
[[ $(awk 'END { print NR }' "$make_log") == 1 ]] || fail "warm call did not perform exactly one build"
[[ -f "$poison_cache/$source_name/.obj/poison.d" ]] || fail "test poison unexpectedly disappeared"
assert_no_internal_debris "$poison_cache"

echo "[9/13] active lock waits for a bounded interval" >&2
active_cache=$(new_cache active-lock)
active_lock=$active_cache/.quickjs-${version}.lock
mkdir -- "$active_lock"
printf 'held-by-test %s\n' "$$" >"$active_lock/owner"
assert_failure "timed out waiting for active oracle lock" env \
    QJS_ORACLE_LOCK_WAIT_SECONDS=1 QJS_ORACLE_CACHE="$active_cache" \
    "$build_script" --source-only
[[ -d "$active_lock" ]] || fail "active lock was removed by a non-owner"
rm -- "$active_lock/owner"
rmdir -- "$active_lock"

echo "[10/13] stale lock and invalid lock wait fail closed" >&2
stale_cache=$(new_cache stale-lock)
stale_lock=$stale_cache/.quickjs-${version}.lock
mkdir -- "$stale_lock"
printf 'dead-owner 99999999\n' >"$stale_lock/owner"
assert_failure "refusing stale oracle lock" env \
    QJS_ORACLE_LOCK_WAIT_SECONDS=09 QJS_ORACLE_CACHE="$stale_cache" \
    "$build_script" --source-only
[[ -d "$stale_lock" && -f "$stale_lock/owner" ]] || fail "stale lock was not preserved fail-closed"
rm -- "$stale_lock/owner"
rmdir -- "$stale_lock"
assert_failure "must be between 0 and 3600 seconds" env \
    QJS_ORACLE_LOCK_WAIT_SECONDS=999999999999999999999999999999999999 \
    QJS_ORACLE_CACHE="$stale_cache" "$build_script" --source-only

echo "[11/13] two concurrent normal calls serialize and each clean-build" >&2
concurrent_cache=$(prime_cache concurrent)
concurrent_log=$test_root/concurrent-make.log
out_a=$test_root/concurrent-a.out
out_b=$test_root/concurrent-b.out
err_a=$test_root/concurrent-a.err
err_b=$test_root/concurrent-b.err
env FAKE_MAKE_SLEEP=1 FAKE_MAKE_LOG="$concurrent_log" MAKE="$fake_make" \
    QJS_ORACLE_LOCK_WAIT_SECONDS=10 QJS_ORACLE_CACHE="$concurrent_cache" \
    "$build_script" >"$out_a" 2>"$err_a" &
pid_a=$!
sleep 0.2
env FAKE_MAKE_SLEEP=1 FAKE_MAKE_LOG="$concurrent_log" MAKE="$fake_make" \
    QJS_ORACLE_LOCK_WAIT_SECONDS=10 QJS_ORACLE_CACHE="$concurrent_cache" \
    "$build_script" >"$out_b" 2>"$err_b" &
pid_b=$!
wait "$pid_a" || { sed -n '1,160p' "$err_a" >&2; fail "first concurrent call failed"; }
wait "$pid_b" || { sed -n '1,160p' "$err_b" >&2; fail "second concurrent call failed"; }
expected_oracle=$concurrent_cache/$source_name/qjs
[[ $(sed -n '1p' "$out_a") == "$expected_oracle" && $(awk 'END { print NR }' "$out_a") == 1 ]] || \
    fail "first concurrent call emitted invalid stdout"
[[ $(sed -n '1p' "$out_b") == "$expected_oracle" && $(awk 'END { print NR }' "$out_b") == 1 ]] || \
    fail "second concurrent call emitted invalid stdout"
[[ $(awk 'END { print NR }' "$concurrent_log") == 2 ]] || fail "concurrent calls did not each clean-build"
assert_no_internal_debris "$concurrent_cache"

echo "[12/13] build failures leave no partial publish" >&2
cold_failure_cache=$(new_cache cold-build-failure)
assert_failure "fake-make-failure" env MAKE="$failing_make" \
    QJS_ORACLE_CACHE="$cold_failure_cache" "$build_script"
[[ ! -e "$cold_failure_cache/$source_name" ]] || fail "cold failed build published a partial source/qjs tree"
assert_no_internal_debris "$cold_failure_cache"

warm_failure_cache=$(prime_cache warm-build-failure)
printf '%s\n' '#!/usr/bin/env sh' '# prior-qjs' 'exit 0' >"$warm_failure_cache/$source_name/qjs"
chmod +x "$warm_failure_cache/$source_name/qjs"
assert_failure "fake-make-failure" env MAKE="$failing_make" \
    QJS_ORACLE_CACHE="$warm_failure_cache" "$build_script"
grep -F '# prior-qjs' "$warm_failure_cache/$source_name/qjs" >/dev/null || \
    fail "failed warm build replaced the prior qjs"
assert_no_internal_debris "$warm_failure_cache"

echo "[13/13] fresh normal build publishes source and qjs without network" >&2
fresh_cache=$(new_cache fresh-normal)
fresh_make_log=$test_root/fresh-make.log
rm -f -- "$offline_marker"
assert_success_path "$fresh_cache/$source_name/qjs" env \
    PATH="$offline_bin:$PATH" FAKE_MAKE_LOG="$fresh_make_log" MAKE="$fake_make" \
    QJS_ORACLE_CACHE="$fresh_cache" "$build_script"
[[ ! -e "$offline_marker" ]] || fail "fresh normal build downloaded despite a verified archive"
[[ -d "$fresh_cache/$source_name" && ! -L "$fresh_cache/$source_name" ]] || \
    fail "fresh normal build did not publish the source directory"
[[ -f "$fresh_cache/$source_name/qjs" && -x "$fresh_cache/$source_name/qjs" && \
   ! -L "$fresh_cache/$source_name/qjs" ]] || fail "fresh normal build did not publish qjs"
[[ $(awk 'END { print NR }' "$fresh_make_log") == 1 ]] || \
    fail "fresh normal call did not perform exactly one clean build"
assert_no_internal_debris "$fresh_cache"

echo "PASS: isolated QuickJS oracle cache hardening" >&2

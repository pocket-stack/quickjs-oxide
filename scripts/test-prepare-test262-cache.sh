#!/usr/bin/env bash
set -euo pipefail

# Isolated regression coverage for prepare-test262.sh cache authentication.

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
version=2026-06-04
source_name=quickjs-${version}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

for command_name in git sed cmp find mkfifo; do
    command -v "$command_name" >/dev/null 2>&1 || fail "$command_name is required"
done

while IFS='=' read -r inherited_name _; do
    case $inherited_name in
        GIT_*) unset "$inherited_name" ;;
    esac
done < <(env)

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

test_root=$(mktemp -d "${TMPDIR:-/tmp}/prepare-test262-cache-test.XXXXXX")
test_root=$(CDPATH='' cd -- "$test_root" && pwd -P)
trap 'rm -rf -- "$test_root"' EXIT HUP INT TERM
real_git=$(command -v git)
sequence=0

origin_work=$test_root/origin-work
origin=$test_root/origin.git
mkdir -- "$origin_work"
git -C "$origin_work" init -q
git -C "$origin_work" config user.name fixture
git -C "$origin_work" config user.email fixture@example.invalid
mkdir -p "$origin_work/harness" "$origin_work/test" "$origin_work/tools"
printf 'let value = "old";\n' >"$origin_work/harness/a.js"
printf 'fixture test\n' >"$origin_work/test/case.js"
mkdir -- "$origin_work/test/nested"
printf 'nested fixture\n' >"$origin_work/test/nested/deep.js"
printf '%s\n' '#!/usr/bin/env sh' 'exit 0' >"$origin_work/tools/run.sh"
chmod +x "$origin_work/tools/run.sh"
git -C "$origin_work" add .
git -C "$origin_work" commit -qm fixture
expected_commit=$(git -C "$origin_work" rev-parse HEAD)
printf 'let value = "new";\n' >"$origin_work/harness/a.js"
patch_fixture=$test_root/test262.patch
git -C "$origin_work" diff --binary -- harness/a.js >"$patch_fixture"
git -C "$origin_work" checkout -q -- harness/a.js
printf 'wrong head\n' >"$origin_work/wrong.txt"
git -C "$origin_work" add wrong.txt
git -C "$origin_work" commit -qm wrong
wrong_commit=$(git -C "$origin_work" rev-parse HEAD)
git clone -q --bare "$origin_work" "$origin"

config_fixture=$test_root/test262.conf
printf 'fixture config\n' >"$config_fixture"
patch_sha=$(sha256_file "$patch_fixture")
config_sha=$(sha256_file "$config_fixture")

program_dir=$test_root/program
mkdir -- "$program_dir"
sed \
    -e "s|^expected_commit=.*|expected_commit=$expected_commit|" \
    -e "s|^expected_patch_sha256=.*|expected_patch_sha256=$patch_sha|" \
    -e "s|^expected_config_sha256=.*|expected_config_sha256=$config_sha|" \
    -e "s|^test262_url=.*|test262_url=$origin|" \
    "$script_dir/prepare-test262.sh" >"$program_dir/prepare-test262.sh"
chmod +x "$program_dir/prepare-test262.sh"

cat >"$program_dir/build-quickjs-oracle.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
source_dir=${QJS_ORACLE_CACHE:?}/quickjs-2026-06-04
if [[ -n ${FAKE_BUILD_CALL_LOG:-} ]]; then
    printf '%s\n' "${1-}" >>"$FAKE_BUILD_CALL_LOG"
fi
case ${1-} in
    --source-only)
        printf '%s\n' "$source_dir"
        ;;
    --test262-oracles)
        if [[ -n ${FAKE_BUILD_LOG:-} ]]; then
            printf 'combined %s\n' "$source_dir" >>"$FAKE_BUILD_LOG"
        fi
        if [[ -n ${FAKE_BUILD_ACTIVE_DIR:-} ]]; then
            if ! mkdir -- "$FAKE_BUILD_ACTIVE_DIR" 2>/dev/null; then
                printf 'overlap\n' >"${FAKE_BUILD_OVERLAP_MARKER:?}"
                exit 70
            fi
            trap 'rmdir -- "$FAKE_BUILD_ACTIVE_DIR" 2>/dev/null || true' EXIT
        fi
        sleep "${FAKE_BUILD_SLEEP:-0}"
        if [[ ${FAKE_BUILD_FAIL:-0} == 1 ]]; then
            echo 'fake combined build failure' >&2
            exit 42
        fi
        qjs_tmp=$source_dir/.qjs.fake.$$
        runner_tmp=$source_dir/.run-test262.fake.$$
        printf '%s\n' '#!/usr/bin/env sh' '# fresh qjs' 'exit 0' >"$qjs_tmp"
        printf '%s\n' '#!/usr/bin/env sh' '# fresh runner' 'exit 0' >"$runner_tmp"
        chmod +x "$qjs_tmp" "$runner_tmp"
        mv -f -- "$qjs_tmp" "$source_dir/qjs"
        mv -f -- "$runner_tmp" "$source_dir/run-test262"
        if [[ -n ${FAKE_BUILD_ACTIVE_DIR:-} ]]; then
            rmdir -- "$FAKE_BUILD_ACTIVE_DIR"
            trap - EXIT
        fi
        printf '%s\n' "$source_dir"
        ;;
    *) exit 64 ;;
esac
EOF
chmod +x "$program_dir/build-quickjs-oracle.sh"

git_bin=$test_root/git-bin
mkdir -- "$git_bin"
git_fetch_marker=$test_root/git-fetch-called
git_call_marker=$test_root/git-called
cat >"$git_bin/git" <<EOF
#!/usr/bin/env bash
printf 'git\n' >>'$git_call_marker'
for argument in "\$@"; do
    if [[ "\$argument" == fetch ]]; then
        printf 'fetch\n' >>'$git_fetch_marker'
    fi
done
exec '$real_git' "\$@"
EOF
chmod +x "$git_bin/git"
test_path=$git_bin:$PATH

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
        sed -n '1,120p' "$stderr_file" >&2
        fail "command unexpectedly failed: $*"
    fi
    lines=$(awk 'END { print NR }' "$stdout_file")
    actual=$(sed -n '1p' "$stdout_file")
    [[ "$lines" == 1 && "$actual" == "$expected" ]] || \
        fail "stdout was not exactly the expected path: $*"
}

assert_failure() {
    local pattern=$1
    shift
    next_result_files
    if "$@" >"$stdout_file" 2>"$stderr_file"; then
        fail "command unexpectedly succeeded: $*"
    fi
    [[ ! -s "$stdout_file" ]] || fail "failed command wrote stdout: $*"
    grep -F "$pattern" "$stderr_file" >/dev/null || {
        sed -n '1,120p' "$stderr_file" >&2
        fail "failure did not contain '$pattern': $*"
    }
}

new_cache() {
    local name=$1
    local cache=$test_root/$name
    mkdir -p "$cache/$source_name/tests"
    cp -- "$patch_fixture" "$cache/$source_name/tests/test262.patch"
    cp -- "$config_fixture" "$cache/$source_name/test262.conf"
    printf '%s\n' "$cache"
}

copy_cache() {
    local name=$1
    local cache=$test_root/$name
    cp -R -- "$base_cache" "$cache"
    printf '%s\n' "$cache"
}

assert_no_debris() {
    local cache=$1
    local debris
    debris=$(find "$cache" -maxdepth 1 \
        \( -name '.test262-work.*' -o -name ".test262-${expected_commit}.lock" \) -print)
    [[ -z "$debris" ]] || fail "prepare left internal debris: $debris"
}

echo "[1/17] unsafe caller Git environment fails before side effects" >&2
caller_env_cache=$(new_cache caller-env)
caller_env_build_log=$test_root/caller-env-build.log
rm -f -- "$git_fetch_marker"
for unsafe_setting in \
    GIT_PAGER= \
    GIT_PAGER=less \
    'GIT_PAGER=cat -n' \
    GIT_DIR=/definitely/wrong \
    GIT_INDEX_FILE=/definitely/wrong-index \
    GIT_CONFIG_COUNT=1 \
    GIT_LITERAL_PATHSPECS=1; do
    unsafe_name=${unsafe_setting%%=*}
    assert_failure "unsafe caller Git environment: $unsafe_name" env \
        "$unsafe_setting" PATH="$test_path" FAKE_BUILD_CALL_LOG="$caller_env_build_log" \
        QJS_ORACLE_CACHE="$caller_env_cache" "$program_dir/prepare-test262.sh"
done
assert_failure "unsafe caller Git environment: GIT_PAGER" env \
    $'GIT_PAGER=cat\nless' PATH="$test_path" FAKE_BUILD_CALL_LOG="$caller_env_build_log" \
    QJS_ORACLE_CACHE="$caller_env_cache" "$program_dir/prepare-test262.sh"
assert_failure "unsafe caller Git environment: GIT_DIR" env \
    GIT_PAGER=cat GIT_DIR=/definitely/wrong PATH="$test_path" \
    FAKE_BUILD_CALL_LOG="$caller_env_build_log" \
    QJS_ORACLE_CACHE="$caller_env_cache" "$program_dir/prepare-test262.sh"
[[ ! -e "$caller_env_cache/$source_name/test262" ]] || fail "unsafe caller environment published suite"
[[ ! -e "$caller_env_build_log" ]] || fail "unsafe caller environment invoked oracle build"
[[ ! -e "$git_fetch_marker" ]] || fail "unsafe caller environment performed network fetch"
assert_no_debris "$caller_env_cache"

echo "[2/17] cold bootstrap and combined build" >&2
base_cache=$(new_cache base)
base_suite=$base_cache/$source_name/test262
base_build_log=$test_root/base-build.log
assert_success_path "$base_suite" env PATH="$test_path" \
    FAKE_BUILD_LOG="$base_build_log" QJS_ORACLE_CACHE="$base_cache" \
    "$program_dir/prepare-test262.sh"
[[ -s "$git_fetch_marker" ]] || fail "cold bootstrap did not fetch the pinned commit"
grep -F 'let value = "new";' "$base_suite/harness/a.js" >/dev/null || \
    fail "cold bootstrap did not apply the verified patch"
[[ -x "$base_cache/$source_name/qjs" && -x "$base_cache/$source_name/run-test262" ]] || \
    fail "prepare did not publish both fresh oracle executables"
[[ $(awk 'END { print NR }' "$base_build_log") == 1 ]] || fail "cold prepare did not combined-build once"
assert_no_debris "$base_cache"

echo "[3/17] warm valid checkout accepts GIT_PAGER=cat and remains offline" >&2
rm -f -- "$git_fetch_marker"
assert_success_path "$base_suite" env GIT_PAGER=cat PATH="$test_path" \
    QJS_ORACLE_CACHE="$base_cache" "$program_dir/prepare-test262.sh"
[[ ! -e "$git_fetch_marker" ]] || fail "warm prepare performed a fetch"
[[ $(env GIT_PAGER=cat "$real_git" -C "$base_suite" config --local --get core.pager) == cat ]] || \
    fail "approved caller pager did not preserve the canonical pager"
[[ $(env GIT_PAGER=cat "$real_git" -C "$base_suite" status --porcelain=v1 --untracked-files=all) == \
   ' M harness/a.js' ]] || fail "approved caller pager changed downstream Git status"
assert_no_debris "$base_cache"

echo "[4/17] canonical index and config replace cached Git control state" >&2
index_cache=$(copy_cache canonical-index)
index_suite=$index_cache/$source_name/test262
"$real_git" -C "$index_suite" rm -q --cached test/case.js
printf 'index-only extra\n' >"$index_suite/index-only.js"
"$real_git" -C "$index_suite" add index-only.js
rm -- "$index_suite/index-only.js"
"$real_git" -C "$index_suite" update-index --assume-unchanged harness/a.js
"$real_git" -C "$index_suite" update-index --skip-worktree harness/a.js
decoy_worktree=$test_root/decoy-worktree
mkdir -p "$decoy_worktree/harness"
printf 'decoy\n' >"$decoy_worktree/harness/a.js"
malicious_include=$test_root/malicious-git-config
printf '%s\n' '[filter "evil"]' '    smudge = printf evil' >"$malicious_include"
default_hook_marker=$test_root/default-hook-ran
custom_hook_marker=$test_root/custom-hook-ran
fsmonitor_marker=$test_root/fsmonitor-ran
custom_hooks=$test_root/custom-hooks
mkdir -- "$custom_hooks"
cat >"$custom_hooks/post-index-change" <<EOF
#!/usr/bin/env bash
printf 'custom hook\n' >'$custom_hook_marker'
EOF
cat >"$test_root/fsmonitor-hook" <<EOF
#!/usr/bin/env bash
printf 'fsmonitor\n' >'$fsmonitor_marker'
exit 1
EOF
chmod +x "$custom_hooks/post-index-change" "$test_root/fsmonitor-hook"
"$real_git" --git-dir="$index_suite/.git" config core.worktree "$decoy_worktree" 2>/dev/null
"$real_git" --git-dir="$index_suite/.git" config core.bare true 2>/dev/null
"$real_git" --git-dir="$index_suite/.git" config include.path "$malicious_include" 2>/dev/null
"$real_git" --git-dir="$index_suite/.git" config filter.local.clean 'printf local-evil' 2>/dev/null
"$real_git" --git-dir="$index_suite/.git" config core.hooksPath "$custom_hooks" 2>/dev/null
"$real_git" --git-dir="$index_suite/.git" config core.fsmonitor "$test_root/fsmonitor-hook" 2>/dev/null
printf 'status-hidden.tmp\n' >"$index_suite/.git/info/exclude"
assert_success_path "$index_suite" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$index_cache" "$program_dir/prepare-test262.sh"
"$real_git" -C "$index_suite" ls-tree -r -z --full-tree \
    --format='%(objectmode) %(objectname) 0%x09%(path)' "$expected_commit" \
    >"$test_root/index.expected"
"$real_git" -C "$index_suite" ls-files --stage -z >"$test_root/index.actual"
cmp -s "$test_root/index.expected" "$test_root/index.actual" || \
    fail "canonical index stage entries differ from pinned commit"
while IFS= read -r -d '' indexed_entry; do
    [[ ${indexed_entry:0:1} == H ]] || fail "canonical index retained entry flags: $indexed_entry"
done < <("$real_git" -C "$index_suite" ls-files -v -z)
[[ $("$real_git" -C "$index_suite" status --porcelain=v1 --untracked-files=all) == ' M harness/a.js' ]] || \
    fail "verified patch did not remain a worktree-only modification"
[[ -f "$index_suite/.git/info/exclude" && ! -L "$index_suite/.git/info/exclude" && \
   ! -s "$index_suite/.git/info/exclude" ]] || \
    fail "canonical info/exclude was not published as an empty regular file"
printf 'visible\n' >"$index_suite/status-hidden.tmp"
"$real_git" -C "$index_suite" status --porcelain=v1 --untracked-files=all | \
    grep -F '?? status-hidden.tmp' >/dev/null || \
    fail "cached info/exclude still hid downstream status entries"
rm -- "$index_suite/status-hidden.tmp"
[[ $("$real_git" -C "$index_suite" rev-parse --show-toplevel) == "$index_suite" ]] || \
    fail "canonical config did not bind downstream Git to the verified worktree"
"$real_git" -C "$index_suite" grep -F 'let value = "new";' -- harness/a.js >/dev/null || \
    fail "downstream Git did not read the verified patched worktree"
[[ $("$real_git" -C "$index_suite" config --local --get core.repositoryformatversion) == 0 ]] || \
    fail "canonical config has wrong repository format"
[[ $("$real_git" -C "$index_suite" config --local --get core.filemode) == true ]] || \
    fail "canonical config did not enable executable-mode checks"
[[ $("$real_git" -C "$index_suite" config --local --get core.bare) == false ]] || \
    fail "canonical config remained bare"
[[ $("$real_git" -C "$index_suite" config --local --get core.worktree) == .. ]] || \
    fail "canonical config did not explicitly bind the worktree"
[[ $("$real_git" -C "$index_suite" config --local --get core.logallrefupdates) == true ]] || \
    fail "canonical config has wrong reflog policy"
[[ $("$real_git" -C "$index_suite" config --local --get core.hookspath) == /dev/null ]] || \
    fail "canonical config did not disable repository hooks"
[[ $("$real_git" -C "$index_suite" config --local --get core.fsmonitor) == false ]] || \
    fail "canonical config did not disable fsmonitor"
[[ $("$real_git" -C "$index_suite" config --local --get core.attributesfile) == /dev/null ]] || \
    fail "canonical config did not disable external attributes"
[[ $("$real_git" -C "$index_suite" config --local --get core.excludesfile) == /dev/null ]] || \
    fail "canonical config did not disable external excludes"
for false_key in sparsecheckout untrackedcache ignorestat; do
    [[ $("$real_git" -C "$index_suite" config --local --get "core.$false_key") == false ]] || \
        fail "canonical config did not disable core.$false_key"
done
[[ $("$real_git" -C "$index_suite" config --local --get core.pager) == cat ]] || \
    fail "canonical config did not fix the pager"
if "$real_git" -C "$index_suite" config --local --get-regexp \
    '^(include\.|filter\.|diff\.|remote\.)' >/dev/null 2>&1; then
    fail "canonical config retained unsafe cached settings"
fi
[[ -x "$index_suite/tools/run.sh" ]] || fail "pinned executable lost its mode"
"$real_git" -C "$index_suite" ls-files >/dev/null
[[ ! -e "$custom_hook_marker" && ! -e "$fsmonitor_marker" ]] || \
    fail "cached custom hook or fsmonitor executed"

default_hook_cache=$(copy_cache default-hook)
default_hook_suite=$default_hook_cache/$source_name/test262
"$real_git" --git-dir="$default_hook_suite/.git" config --unset-all core.hooksPath 2>/dev/null || true
cat >"$default_hook_suite/.git/hooks/post-index-change" <<EOF
#!/usr/bin/env bash
printf 'default hook\n' >'$default_hook_marker'
EOF
chmod +x "$default_hook_suite/.git/hooks/post-index-change"
"$real_git" --git-dir="$default_hook_suite/.git" config core.fsmonitor "$test_root/fsmonitor-hook"
assert_success_path "$default_hook_suite" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$default_hook_cache" "$program_dir/prepare-test262.sh"
"$real_git" -C "$default_hook_suite" status --porcelain=v1 >/dev/null
"$real_git" -C "$default_hook_suite" ls-files >/dev/null
"$real_git" -C "$default_hook_suite" grep -F 'let value = "new";' -- harness/a.js >/dev/null
[[ ! -e "$default_hook_marker" && ! -e "$fsmonitor_marker" ]] || \
    fail "default hook or fsmonitor executed"

global_home=$test_root/global-home
mkdir -- "$global_home"
HOME="$global_home" "$real_git" config --global core.worktree "$decoy_worktree"
HOME="$global_home" "$real_git" config --global core.bare true
HOME="$global_home" "$real_git" config --global core.hooksPath "$custom_hooks"
HOME="$global_home" "$real_git" config --global core.fsmonitor "$test_root/fsmonitor-hook"
HOME="$global_home" "$real_git" config --global core.attributesFile "$malicious_include"
HOME="$global_home" "$real_git" config --global core.excludesFile "$malicious_include"
[[ $(HOME="$global_home" "$real_git" -C "$default_hook_suite" rev-parse --show-toplevel) == "$default_hook_suite" ]] || \
    fail "global Git config redirected the canonical worktree"
HOME="$global_home" "$real_git" -C "$default_hook_suite" status --porcelain=v1 >/dev/null
HOME="$global_home" "$real_git" -C "$default_hook_suite" ls-files >/dev/null
HOME="$global_home" "$real_git" -C "$default_hook_suite" grep -F \
    'let value = "new";' -- harness/a.js >/dev/null
[[ ! -e "$custom_hook_marker" && ! -e "$fsmonitor_marker" ]] || \
    fail "global hook or fsmonitor overrode canonical local config"

unsafe_control_build_log=$test_root/unsafe-control-build.log
for unsafe_index_kind in symlink directory fifo lock; do
    unsafe_index_cache=$(copy_cache index-$unsafe_index_kind)
    unsafe_index_suite=$unsafe_index_cache/$source_name/test262
    case $unsafe_index_kind in
        symlink)
            rm -- "$unsafe_index_suite/.git/index"
            ln -s HEAD "$unsafe_index_suite/.git/index"
            expected_index_error='refusing unsafe Test262 index'
            ;;
        directory)
            rm -- "$unsafe_index_suite/.git/index"
            mkdir -- "$unsafe_index_suite/.git/index"
            expected_index_error='refusing unsafe Test262 index'
            ;;
        fifo)
            rm -- "$unsafe_index_suite/.git/index"
            mkfifo "$unsafe_index_suite/.git/index"
            expected_index_error='refusing unsafe Test262 index'
            ;;
        lock)
            printf 'occupied\n' >"$unsafe_index_suite/.git/index.lock"
            expected_index_error='refusing existing Test262 index lock'
            ;;
    esac
    assert_failure "$expected_index_error" env PATH="$test_path" \
        FAKE_BUILD_LOG="$unsafe_control_build_log" \
        QJS_ORACLE_CACHE="$unsafe_index_cache" "$program_dir/prepare-test262.sh"
done

for unsafe_config_kind in symlink directory fifo lock worktree; do
    unsafe_config_cache=$(copy_cache config-$unsafe_config_kind)
    unsafe_config_suite=$unsafe_config_cache/$source_name/test262
    case $unsafe_config_kind in
        symlink)
            rm -- "$unsafe_config_suite/.git/config"
            ln -s HEAD "$unsafe_config_suite/.git/config"
            expected_config_error='refusing unsafe Test262 config'
            ;;
        directory)
            rm -- "$unsafe_config_suite/.git/config"
            mkdir -- "$unsafe_config_suite/.git/config"
            expected_config_error='refusing unsafe Test262 config'
            ;;
        fifo)
            rm -- "$unsafe_config_suite/.git/config"
            mkfifo "$unsafe_config_suite/.git/config"
            expected_config_error='refusing unsafe Test262 config'
            ;;
        lock)
            printf 'occupied\n' >"$unsafe_config_suite/.git/config.lock"
            expected_config_error='refusing existing Test262 config lock'
            ;;
        worktree)
            printf '[core]\n' >"$unsafe_config_suite/.git/config.worktree"
            expected_config_error='refusing Test262 worktree config'
            ;;
    esac
    assert_failure "$expected_config_error" env PATH="$test_path" \
        FAKE_BUILD_LOG="$unsafe_control_build_log" \
        QJS_ORACLE_CACHE="$unsafe_config_cache" "$program_dir/prepare-test262.sh"
done

for unsafe_exclude_kind in symlink directory fifo lock; do
    unsafe_exclude_cache=$(copy_cache exclude-$unsafe_exclude_kind)
    unsafe_exclude_suite=$unsafe_exclude_cache/$source_name/test262
    case $unsafe_exclude_kind in
        symlink)
            rm -- "$unsafe_exclude_suite/.git/info/exclude"
            ln -s ../HEAD "$unsafe_exclude_suite/.git/info/exclude"
            expected_exclude_error='refusing unsafe Test262 exclude'
            ;;
        directory)
            rm -- "$unsafe_exclude_suite/.git/info/exclude"
            mkdir -- "$unsafe_exclude_suite/.git/info/exclude"
            expected_exclude_error='refusing unsafe Test262 exclude'
            ;;
        fifo)
            rm -- "$unsafe_exclude_suite/.git/info/exclude"
            mkfifo "$unsafe_exclude_suite/.git/info/exclude"
            expected_exclude_error='refusing unsafe Test262 exclude'
            ;;
        lock)
            printf 'occupied\n' >"$unsafe_exclude_suite/.git/info/exclude.lock"
            expected_exclude_error='refusing existing Test262 exclude lock'
            ;;
    esac
    assert_failure "$expected_exclude_error" env PATH="$test_path" \
        FAKE_BUILD_LOG="$unsafe_control_build_log" \
        QJS_ORACLE_CACHE="$unsafe_exclude_cache" "$program_dir/prepare-test262.sh"
done
[[ ! -e "$unsafe_control_build_log" ]] || fail "unsafe index/config/exclude invoked combined build"

external_git_cache=$(copy_cache external-git-symlink)
external_git_suite=$external_git_cache/$source_name/test262
external_git_dir=$test_root/external-git-dir
mv -- "$external_git_suite/.git" "$external_git_dir"
ln -s "$external_git_dir" "$external_git_suite/.git"
external_config_before=$(sha256_file "$external_git_dir/config")
assert_failure "not a regular pinned git checkout" env PATH="$test_path" \
    FAKE_BUILD_LOG="$unsafe_control_build_log" \
    QJS_ORACLE_CACHE="$external_git_cache" "$program_dir/prepare-test262.sh"
[[ $(sha256_file "$external_git_dir/config") == "$external_config_before" ]] || \
    fail "external config target was mutated through .git symlink"
non_git_cache=$(copy_cache non-git-suite)
mv -- "$non_git_cache/$source_name/test262/.git" "$test_root/non-git-metadata"
assert_failure "not a regular pinned git checkout" env PATH="$test_path" \
    FAKE_BUILD_LOG="$unsafe_control_build_log" \
    QJS_ORACLE_CACHE="$non_git_cache" "$program_dir/prepare-test262.sh"
[[ ! -e "$unsafe_control_build_log" ]] || fail "unsafe suite metadata invoked combined build"

echo "[5/17] same-size restored-mtime tamper bypasses index flags but not byte auth" >&2
tamper_cache=$(copy_cache same-size-tamper)
tamper_suite=$tamper_cache/$source_name/test262
cp -p -- "$tamper_suite/harness/a.js" "$test_root/a.reference"
printf 'let value = "bad";\n' >"$tamper_suite/harness/a.js"
touch -r "$test_root/a.reference" "$tamper_suite/harness/a.js"
"$real_git" -C "$tamper_suite" update-index --assume-unchanged harness/a.js
"$real_git" -C "$tamper_suite" update-index --skip-worktree harness/a.js
assert_failure "paths or bytes differ" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$tamper_cache" "$program_dir/prepare-test262.sh"

echo "[6/17] wrong and unsafe HEAD fail before Git or build" >&2
unsafe_head_build_log=$test_root/unsafe-head-build.log
for unsafe_head_kind in wrong symlink fifo lock; do
    unsafe_head_cache=$(copy_cache head-$unsafe_head_kind)
    unsafe_head_suite=$unsafe_head_cache/$source_name/test262
    case $unsafe_head_kind in
        wrong)
            printf '%s\n' "$wrong_commit" >"$unsafe_head_suite/.git/HEAD"
            expected_head_error='not at the pinned commit'
            ;;
        symlink)
            rm -- "$unsafe_head_suite/.git/HEAD"
            ln -s config "$unsafe_head_suite/.git/HEAD"
            expected_head_error='refusing unsafe Test262 HEAD'
            ;;
        fifo)
            rm -- "$unsafe_head_suite/.git/HEAD"
            mkfifo "$unsafe_head_suite/.git/HEAD"
            expected_head_error='refusing unsafe Test262 HEAD'
            ;;
        lock)
            printf 'occupied\n' >"$unsafe_head_suite/.git/HEAD.lock"
            expected_head_error='refusing existing Test262 HEAD lock'
            ;;
    esac
    rm -f -- "$git_call_marker"
    assert_failure "$expected_head_error" env PATH="$test_path" \
        FAKE_BUILD_LOG="$unsafe_head_build_log" \
        QJS_ORACLE_CACHE="$unsafe_head_cache" "$program_dir/prepare-test262.sh"
    [[ ! -e "$git_call_marker" ]] || fail "unsafe HEAD reached Git: $unsafe_head_kind"
done
[[ ! -e "$unsafe_head_build_log" ]] || fail "unsafe HEAD invoked combined build"

echo "[7/17] missing, extra newline-name, symlink, and FIFO members are rejected" >&2
missing_cache=$(copy_cache missing)
rm -- "$missing_cache/$source_name/test262/test/case.js"
assert_failure "paths or bytes differ" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$missing_cache" "$program_dir/prepare-test262.sh"
extra_cache=$(copy_cache extra-newline)
printf 'extra\n' >"$extra_cache/$source_name/test262/extra
name"
assert_failure "paths or bytes differ" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$extra_cache" "$program_dir/prepare-test262.sh"
symlink_cache=$(copy_cache symlink)
rm -- "$symlink_cache/$source_name/test262/test/case.js"
ln -s ../harness/a.js "$symlink_cache/$source_name/test262/test/case.js"
assert_failure "symlink or special" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$symlink_cache" "$program_dir/prepare-test262.sh"
fifo_cache=$(copy_cache fifo)
mkfifo "$fifo_cache/$source_name/test262/unsafe-fifo"
assert_failure "symlink or special" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$fifo_cache" "$program_dir/prepare-test262.sh"

echo "[8/17] executable mode changes are rejected" >&2
mode_cache=$(copy_cache mode)
chmod -x "$mode_cache/$source_name/test262/tools/run.sh"
assert_failure "executable mode differs" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$mode_cache" "$program_dir/prepare-test262.sh"

echo "[9/17] bad patch and config checksums are rejected" >&2
patch_cache=$(copy_cache bad-patch)
printf 'bad patch\n' >>"$patch_cache/$source_name/tests/test262.patch"
assert_failure "patch checksum mismatch" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$patch_cache" "$program_dir/prepare-test262.sh"
config_cache=$(copy_cache bad-config)
printf 'bad config\n' >>"$config_cache/$source_name/test262.conf"
assert_failure "config checksum mismatch" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$config_cache" "$program_dir/prepare-test262.sh"

echo "[10/17] replace refs, alternates, grafts, and info attributes fail closed" >&2
replace_cache=$(copy_cache replace-ref)
"$real_git" -C "$replace_cache/$source_name/test262" fetch -q "$origin" "$wrong_commit"
"$real_git" -C "$replace_cache/$source_name/test262" replace "$expected_commit" "$wrong_commit"
assert_failure "replacement refs" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$replace_cache" "$program_dir/prepare-test262.sh"
replace_symlink_cache=$(copy_cache replace-ref-symlink)
mkdir -p "$replace_symlink_cache/$source_name/test262/.git/refs/replace"
ln -s ../../HEAD \
    "$replace_symlink_cache/$source_name/test262/.git/refs/replace/$expected_commit"
assert_failure "replacement refs" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$replace_symlink_cache" "$program_dir/prepare-test262.sh"
alternate_cache=$(copy_cache alternates)
printf '%s\n' "$origin/objects" >"$alternate_cache/$source_name/test262/.git/objects/info/alternates"
assert_failure "unsafe Test262 git metadata" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$alternate_cache" "$program_dir/prepare-test262.sh"
graft_cache=$(copy_cache grafts)
printf '%s\n' "$expected_commit" >"$graft_cache/$source_name/test262/.git/info/grafts"
assert_failure "unsafe Test262 git metadata" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$graft_cache" "$program_dir/prepare-test262.sh"
attrs_cache=$(copy_cache info-attributes)
printf '* filter=evil\n' >"$attrs_cache/$source_name/test262/.git/info/attributes"
"$real_git" -C "$attrs_cache/$source_name/test262" config filter.evil.smudge 'printf evil'
assert_failure "unsafe Test262 git metadata" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$attrs_cache" "$program_dir/prepare-test262.sh"

echo "[11/17] nested .git paths are rejected" >&2
nested_cache=$(copy_cache nested-git)
mkdir -p "$nested_cache/$source_name/test262/test/nested/.git"
printf 'nested\n' >"$nested_cache/$source_name/test262/test/nested/.git/config"
assert_failure "nested .git path" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$nested_cache" "$program_dir/prepare-test262.sh"

echo "[12/17] active and stale locks fail closed" >&2
active_cache=$(copy_cache active-lock)
active_lock=$active_cache/.test262-${expected_commit}.lock
mkdir -- "$active_lock"
printf 'active %s\n' "$$" >"$active_lock/owner"
assert_failure "timed out waiting for active Test262 lock" env PATH="$test_path" \
    QJS_TEST262_LOCK_WAIT_SECONDS=1 QJS_ORACLE_CACHE="$active_cache" \
    "$program_dir/prepare-test262.sh"
[[ -d "$active_lock" ]] || fail "active lock was removed by non-owner"
rm -- "$active_lock/owner"; rmdir -- "$active_lock"
stale_cache=$(copy_cache stale-lock)
stale_lock=$stale_cache/.test262-${expected_commit}.lock
mkdir -- "$stale_lock"
printf 'stale 99999999\n' >"$stale_lock/owner"
assert_failure "refusing stale Test262 lock" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$stale_cache" "$program_dir/prepare-test262.sh"
[[ -d "$stale_lock" ]] || fail "stale lock was removed"
rm -- "$stale_lock/owner"; rmdir -- "$stale_lock"

echo "[13/17] two warm calls serialize" >&2
concurrent_cache=$(copy_cache concurrent)
concurrent_suite=$concurrent_cache/$source_name/test262
concurrent_log=$test_root/concurrent-build.log
concurrent_active=$test_root/concurrent-build-active
concurrent_overlap=$test_root/concurrent-build-overlap
for suffix in a b; do
    env PATH="$test_path" FAKE_BUILD_SLEEP=1 FAKE_BUILD_LOG="$concurrent_log" \
        FAKE_BUILD_ACTIVE_DIR="$concurrent_active" \
        FAKE_BUILD_OVERLAP_MARKER="$concurrent_overlap" \
        QJS_TEST262_LOCK_WAIT_SECONDS=10 QJS_ORACLE_CACHE="$concurrent_cache" \
        "$program_dir/prepare-test262.sh" >"$test_root/concurrent.$suffix.out" \
        2>"$test_root/concurrent.$suffix.err" &
    if [[ $suffix == a ]]; then pid_a=$!; sleep 0.2; else pid_b=$!; fi
done
wait "$pid_a" || fail "first concurrent prepare failed"
wait "$pid_b" || fail "second concurrent prepare failed"
for suffix in a b; do
    [[ $(sed -n '1p' "$test_root/concurrent.$suffix.out") == "$concurrent_suite" && \
       $(awk 'END { print NR }' "$test_root/concurrent.$suffix.out") == 1 ]] || \
        fail "concurrent prepare stdout invalid"
done
[[ $(awk 'END { print NR }' "$concurrent_log") == 2 ]] || fail "concurrent prepares did not serialize two builds"
[[ ! -e "$concurrent_overlap" ]] || fail "concurrent combined builds overlapped"
assert_no_debris "$concurrent_cache"

echo "[14/17] clone and build failure cleanup" >&2
clone_failure_cache=$(new_cache clone-failure)
clone_failure_program=$test_root/clone-failure-program
mkdir -- "$clone_failure_program"
sed "s|^test262_url=.*|test262_url=$test_root/missing-origin.git|" \
    "$program_dir/prepare-test262.sh" >"$clone_failure_program/prepare-test262.sh"
cp -- "$program_dir/build-quickjs-oracle.sh" "$clone_failure_program/build-quickjs-oracle.sh"
chmod +x "$clone_failure_program/prepare-test262.sh" "$clone_failure_program/build-quickjs-oracle.sh"
assert_failure "does not appear to be a git repository" env PATH="$test_path" QJS_ORACLE_CACHE="$clone_failure_cache" \
    "$clone_failure_program/prepare-test262.sh"
[[ ! -e "$clone_failure_cache/$source_name/test262" ]] || fail "clone failure published suite"
assert_no_debris "$clone_failure_cache"
build_failure_cache=$(new_cache build-failure)
assert_failure "fake combined build failure" env PATH="$test_path" FAKE_BUILD_FAIL=1 \
    QJS_ORACLE_CACHE="$build_failure_cache" "$program_dir/prepare-test262.sh"
[[ ! -e "$build_failure_cache/$source_name/test262" ]] || fail "build failure published cold suite"
assert_no_debris "$build_failure_cache"

echo "[15/17] relative cache output remains compatible" >&2
relative_cache=$(new_cache relative-cache)
(CDPATH='' cd -- "$test_root" && \
    assert_success_path "$relative_cache/$source_name/test262" env PATH="$test_path" \
        QJS_ORACLE_CACHE=relative-cache "$program_dir/prepare-test262.sh")

echo "[16/17] poisoned cached binaries and build extras do not bypass combined build" >&2
poison_cache=$(copy_cache poison-build)
poison_source=$poison_cache/$source_name
mkdir -p "$poison_source/.obj" "$poison_source/unicode"
printf 'poison\n' >"$poison_source/.obj/poison.d"
printf 'poison\n' >"$poison_source/unicode/UnicodeData.txt"
printf 'old qjs\n' >"$poison_source/qjs"
printf 'old runner\n' >"$poison_source/run-test262"
chmod +x "$poison_source/qjs" "$poison_source/run-test262"
assert_success_path "$poison_source/test262" env PATH="$test_path" \
    QJS_ORACLE_CACHE="$poison_cache" "$program_dir/prepare-test262.sh"
grep -F '# fresh qjs' "$poison_source/qjs" >/dev/null || fail "prepare reused poisoned qjs"
grep -F '# fresh runner' "$poison_source/run-test262" >/dev/null || fail "prepare reused poisoned runner"
[[ -e "$poison_source/.obj/poison.d" && -e "$poison_source/unicode/UnicodeData.txt" ]] || \
    fail "prepare unexpectedly rewrote persistent build extras"

echo "[17/17] static upstream-oracle consumer audit" >&2
run_file_count=0
run_call_count=0
while IFS= read -r consumer; do
    case $consumer in
        */build-quickjs-oracle.sh|*/prepare-test262.sh|*/test-build-quickjs-oracle-cache.sh|\
        */test-prepare-test262-cache.sh) continue ;;
    esac
    run_file_count=$((run_file_count + 1))
    call_count=$(grep -oF './run-test262' "$consumer" | wc -l | awk '{print $1}')
    run_call_count=$((run_call_count + call_count))
    if ! grep -F 'prepare-test262.sh' "$consumer" >/dev/null; then
        fail "direct ./run-test262 consumer lacks prepare gate: $consumer"
    fi
done < <(grep -lF './run-test262' "$root"/scripts/*.sh | LC_ALL=C sort)
[[ $run_file_count -eq 89 && $run_call_count -eq 94 ]] || \
    fail "direct ./run-test262 inventory drifted: files=$run_file_count calls=$run_call_count"

qjs_file_count=0
while IFS= read -r consumer; do
    case $consumer in
        */build-quickjs-oracle.sh|*/test-build-quickjs-oracle-cache.sh|\
        */test-prepare-test262-cache.sh) continue ;;
    esac
    qjs_file_count=$((qjs_file_count + 1))
    grep -F 'prepare-test262.sh' "$consumer" >/dev/null || \
        fail "source_dir/qjs consumer lacks prepare gate: $consumer"
done < <(grep -lF "\$source_dir/qjs" "$root"/scripts/*.sh | LC_ALL=C sort)
[[ $qjs_file_count -eq 4 ]] || fail "source_dir/qjs consumer inventory drifted: $qjs_file_count"

grep -F -- '--test262-oracles' "$root/scripts/test-host-gc-reentrant-oracle.sh" >/dev/null || \
    fail "host-gc upstream runner consumer lacks combined gate"

while IFS= read -r consumer; do
    case $consumer in
        */build-quickjs-oracle.sh|*/test-build-quickjs-oracle-cache.sh|\
        */test-prepare-test262-cache.sh) continue ;;
    esac
    if grep -E 'make[^#]*-B|-B[^#]*make' "$consumer" >/dev/null; then
        fail "upstream runner consumer still has unconditional make -B: $consumer"
    fi
    if ! awk '
        /\[\[ ! -x .*(runner|run-test262)/ { in_missing_guard = 1 }
        /^[[:space:]]*fi([[:space:]]|$)/ { in_missing_guard = 0 }
        /\$\{MAKE:-make\}.*run-test262/ {
            same_line = ($0 ~ /\[\[ -x .*(runner|run-test262)/ && $0 ~ /\|\|/)
            continued = (previous ~ /\[\[ -x .*(runner|run-test262)/ && $0 ~ /^[[:space:]]*\|\|/)
            if (!same_line && !continued && !in_missing_guard) {
                printf "%s:%d: unguarded upstream runner make\n", FILENAME, FNR > "/dev/stderr"
                bad = 1
            }
        }
        { previous = $0 }
        END { exit bad }
    ' "$consumer"; then
        fail "ordinary upstream runner make is not executable-gated: $consumer"
    fi
done < <(grep -lE '\$\{MAKE:-make\}.*run-test262' "$root"/scripts/*.sh | LC_ALL=C sort)

echo "PASS: isolated Test262 prepare cache authentication" >&2

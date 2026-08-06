#!/usr/bin/env bash
# Prepare and authenticate the exact Test262 checkout used by the QuickJS oracle.

set -euo pipefail

unsafe_git_name=
while IFS='=' read -r environment_name _; do
    case $environment_name in
        GIT_PAGER)
            environment_value=${!environment_name-}
            if [[ "$environment_value" != cat || "${GIT_PAGER-}" != cat ]]; then
                unsafe_git_name=$environment_name
                break
            fi
            ;;
        GIT_*)
            unsafe_git_name=$environment_name
            break
            ;;
    esac
done < <(env)
if [[ -n "$unsafe_git_name" ]]; then
    echo "error: unsafe caller Git environment: $unsafe_git_name" >&2
    exit 2
fi

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
expected_commit=5c8206929d81b2d3d727ca6aac56c18358c8d790
expected_patch_sha256=f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3
expected_config_sha256=79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b
test262_url=https://github.com/tc39/test262.git
lock_wait_seconds=${QJS_TEST262_LOCK_WAIT_SECONDS:-30}

case $lock_wait_seconds in
    ""|*[!0-9]*)
        echo "error: QJS_TEST262_LOCK_WAIT_SECONDS must be between 0 and 3600 seconds" >&2
        exit 2
        ;;
esac
while [[ $lock_wait_seconds == 0?* ]]; do
    lock_wait_seconds=${lock_wait_seconds#0}
done
case $lock_wait_seconds in
    [0-9]|[0-9][0-9]|[0-9][0-9][0-9]|[12][0-9][0-9][0-9]|3[0-5][0-9][0-9]|3600) ;;
    *)
        echo "error: QJS_TEST262_LOCK_WAIT_SECONDS must be between 0 and 3600 seconds" >&2
        exit 2
        ;;
esac
lock_wait_seconds=$((10#$lock_wait_seconds))

for required_command in git tar diff find cmp; do
    command -v "$required_command" >/dev/null 2>&1 || {
        echo "error: $required_command is required to prepare the Test262 oracle" >&2
        exit 2
    }
done

source_output=$("$script_dir/build-quickjs-oracle.sh" --source-only)
if [[ -z "$source_output" || "$source_output" == *$'\n'* ]]; then
    echo "error: build-quickjs-oracle.sh --source-only must print exactly one path" >&2
    exit 1
fi
source_dir=$source_output
if [[ "$source_dir" != /* ]]; then
    source_dir=$(CDPATH='' cd -- "$(dirname -- "$source_dir")" && pwd)/$(basename -- "$source_dir")
fi

suite=$source_dir/test262
patch=$source_dir/tests/test262.patch
config=$source_dir/test262.conf
cache=$(CDPATH='' cd -- "$source_dir/.." && pwd)
lock_dir=$cache/.test262-${expected_commit}.lock

if [[ ! -f "$patch" || -L "$patch" || ! -f "$config" || -L "$config" ]]; then
    echo "error: pinned QuickJS Test262 patch or config is missing or unsafe" >&2
    exit 1
fi

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required to verify Test262 inputs" >&2
        exit 2
    fi
}

actual_patch_sha256=$(sha256_file "$patch")
actual_config_sha256=$(sha256_file "$config")
if [[ "$actual_patch_sha256" != "$expected_patch_sha256" ]]; then
    echo "error: QuickJS Test262 patch checksum mismatch" >&2
    echo "expected: $expected_patch_sha256" >&2
    echo "actual:   $actual_patch_sha256" >&2
    exit 1
fi
if [[ "$actual_config_sha256" != "$expected_config_sha256" ]]; then
    echo "error: QuickJS Test262 config checksum mismatch" >&2
    echo "expected: $expected_config_sha256" >&2
    echo "actual:   $actual_config_sha256" >&2
    exit 1
fi

lock_token="$$.$(date +%s).${RANDOM-0}"
lock_owned=0
work_dir=$cache/.test262-work.$lock_token
index_tmp=
config_tmp=
exclude_tmp=

release_lock() {
    if [[ $lock_owned -eq 1 && -d "$lock_dir" && ! -L "$lock_dir" && \
          -f "$lock_dir/owner" && ! -L "$lock_dir/owner" ]]; then
        current_owner=$(sed -n '1p' "$lock_dir/owner" 2>/dev/null || true)
        if [[ "$current_owner" == "$lock_token $$" ]]; then
            rm -f -- "$lock_dir/owner"
            rmdir -- "$lock_dir" 2>/dev/null || true
        fi
    fi
}

cleanup() {
    if [[ -n "$index_tmp" ]]; then
        rm -f -- "$index_tmp" 2>/dev/null || true
    fi
    if [[ -n "$config_tmp" ]]; then
        rm -f -- "$config_tmp" 2>/dev/null || true
    fi
    if [[ -n "$exclude_tmp" ]]; then
        rm -f -- "$exclude_tmp" 2>/dev/null || true
    fi
    rm -rf -- "$work_dir" 2>/dev/null || true
    release_lock
}

validate_canonical_config_target() {
    local actual=$1
    local canonical_head=$actual/.git/HEAD
    local head_lock=$actual/.git/HEAD.lock
    local info_dir=$actual/.git/info
    local canonical_exclude=$info_dir/exclude
    local exclude_lock=$info_dir/exclude.lock
    local canonical_config=$actual/.git/config
    local config_lock=$actual/.git/config.lock
    local worktree_config=$actual/.git/config.worktree
    local head_line head_valid REPLY

    if [[ ! -d "$actual" || -L "$actual" || ! -d "$actual/.git" || -L "$actual/.git" ]]; then
        echo "error: Test262 path is not a regular pinned git checkout: $actual" >&2
        return 1
    fi
    if [[ ! -d "$info_dir" || -L "$info_dir" ]]; then
        echo "error: refusing unsafe Test262 git metadata directory: $info_dir" >&2
        return 1
    fi
    if [[ -e "$head_lock" || -L "$head_lock" ]]; then
        echo "error: refusing existing Test262 HEAD lock: $head_lock" >&2
        return 1
    fi
    if [[ ! -f "$canonical_head" || -L "$canonical_head" ]]; then
        echo "error: refusing unsafe Test262 HEAD: $canonical_head" >&2
        return 1
    fi
    head_valid=1
    head_line=
    {
        if ! IFS= read -r head_line; then
            head_valid=0
        elif IFS= read -r -n 1; then
            head_valid=0
        fi
    } < "$canonical_head"
    if [[ $head_valid -ne 1 || "$head_line" != "$expected_commit" ]]; then
        echo "error: Test262 checkout is not at the pinned commit" >&2
        echo "expected: $expected_commit" >&2
        echo "actual:   $head_line" >&2
        return 1
    fi
    if [[ -e "$exclude_lock" || -L "$exclude_lock" ]]; then
        echo "error: refusing existing Test262 exclude lock: $exclude_lock" >&2
        return 1
    fi
    if [[ -e "$canonical_exclude" || -L "$canonical_exclude" ]]; then
        if [[ ! -f "$canonical_exclude" || -L "$canonical_exclude" ]]; then
            echo "error: refusing unsafe Test262 exclude: $canonical_exclude" >&2
            return 1
        fi
    fi
    if [[ -e "$config_lock" || -L "$config_lock" ]]; then
        echo "error: refusing existing Test262 config lock: $config_lock" >&2
        return 1
    fi
    if [[ -e "$worktree_config" || -L "$worktree_config" ]]; then
        echo "error: refusing Test262 worktree config: $worktree_config" >&2
        return 1
    fi
    if [[ -e "$canonical_config" || -L "$canonical_config" ]]; then
        if [[ ! -f "$canonical_config" || -L "$canonical_config" ]]; then
            echo "error: refusing unsafe Test262 config: $canonical_config" >&2
            return 1
        fi
    fi
}

publish_canonical_exclude() {
    local actual=$1
    local canonical_exclude=$actual/.git/info/exclude

    exclude_tmp=$actual/.git/info/.exclude.$lock_token.tmp
    if [[ -e "$exclude_tmp" || -L "$exclude_tmp" ]]; then
        echo "error: refusing occupied Test262 temporary exclude path: $exclude_tmp" >&2
        return 1
    fi
    : > "$exclude_tmp"
    if [[ ! -f "$exclude_tmp" || -L "$exclude_tmp" ]]; then
        echo "error: failed to create a regular canonical Test262 exclude" >&2
        return 1
    fi
    mv -f -- "$exclude_tmp" "$canonical_exclude"
    exclude_tmp=
    if [[ ! -f "$canonical_exclude" || -L "$canonical_exclude" ]]; then
        echo "error: published Test262 exclude is not a regular file" >&2
        return 1
    fi
}

publish_canonical_config() {
    local actual=$1
    local canonical_config=$actual/.git/config

    config_tmp=$actual/.git/.config.$lock_token.tmp
    if [[ -e "$config_tmp" || -L "$config_tmp" ]]; then
        echo "error: refusing occupied Test262 temporary config path: $config_tmp" >&2
        return 1
    fi
    printf '%s\n' \
        '[core]' \
        '    repositoryformatversion = 0' \
        '    filemode = true' \
        '    bare = false' \
        '    worktree = ..' \
        '    logallrefupdates = true' \
        '    hooksPath = /dev/null' \
        '    fsmonitor = false' \
        '    attributesFile = /dev/null' \
        '    excludesFile = /dev/null' \
        '    sparseCheckout = false' \
        '    untrackedCache = false' \
        '    ignoreStat = false' \
        '    pager = cat' > "$config_tmp"
    if [[ ! -f "$config_tmp" || -L "$config_tmp" ]]; then
        echo "error: failed to create a regular canonical Test262 config" >&2
        return 1
    fi
    mv -f -- "$config_tmp" "$canonical_config"
    config_tmp=
    if [[ ! -f "$canonical_config" || -L "$canonical_config" ]]; then
        echo "error: published Test262 config is not a regular file" >&2
        return 1
    fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

lock_deadline=$(( $(date +%s) + lock_wait_seconds ))
while ! mkdir -- "$lock_dir" 2>/dev/null; do
    if [[ ! -d "$lock_dir" || -L "$lock_dir" ]]; then
        echo "error: Test262 lock path is not a directory: $lock_dir" >&2
        exit 1
    fi
    if [[ -e "$lock_dir/owner" || -L "$lock_dir/owner" ]]; then
        if [[ ! -f "$lock_dir/owner" || -L "$lock_dir/owner" ]]; then
            echo "error: refusing unsafe Test262 lock owner: $lock_dir/owner" >&2
            exit 1
        fi
        owner_line=$(sed -n '1p' "$lock_dir/owner" 2>/dev/null || true)
        owner_pid=${owner_line#* }
        owner_token=${owner_line%% *}
        case $owner_pid in
            ""|*[!0-9]*)
                echo "error: refusing malformed or stale Test262 lock: $lock_dir" >&2
                exit 1
                ;;
        esac
        if [[ -z "$owner_token" || "$owner_token" == "$owner_line" ]] || \
           ! kill -0 "$owner_pid" 2>/dev/null; then
            echo "error: refusing stale Test262 lock: $lock_dir" >&2
            exit 1
        fi
    fi
    if [[ $(date +%s) -ge $lock_deadline ]]; then
        echo "error: timed out waiting for active Test262 lock: $lock_dir" >&2
        exit 1
    fi
    sleep 1
done

umask 077
printf '%s %s\n' "$lock_token" "$$" > "$lock_dir/owner"
lock_owned=1
mkdir -- "$work_dir" "$work_dir/git-home" "$work_dir/git-config"

trusted_git() {
    local controlled_index proxy_name proxy_value
    local -a clean_env
    controlled_index=
    if [[ ${1-} == --controlled-index ]]; then
        controlled_index=$2
        shift 2
    fi
    clean_env=(env -i
        "PATH=$PATH"
        "HOME=$work_dir/git-home"
        "XDG_CONFIG_HOME=$work_dir/git-config"
        "TMPDIR=${TMPDIR:-/tmp}"
        "LC_ALL=C"
        "GIT_CONFIG_NOSYSTEM=1"
        "GIT_NO_REPLACE_OBJECTS=1"
        "GIT_NO_LAZY_FETCH=1"
        "GIT_ATTR_NOSYSTEM=1"
        "GIT_CEILING_DIRECTORIES=$work_dir"
        "GIT_TERMINAL_PROMPT=0")
    if [[ -n "$controlled_index" ]]; then
        clean_env+=("GIT_INDEX_FILE=$controlled_index")
    fi
    for proxy_name in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY \
        NO_PROXY no_proxy SSL_CERT_FILE SSL_CERT_DIR GIT_SSL_CAINFO; do
        proxy_value=${!proxy_name-}
        if [[ -n "$proxy_value" ]]; then
            clean_env+=("$proxy_name=$proxy_value")
        fi
    done
    "${clean_env[@]}" git --no-replace-objects \
        -c core.hooksPath=/dev/null \
        -c core.fsmonitor=false \
        -c core.attributesFile=/dev/null \
        "$@"
}

publish_canonical_index() {
    local actual=$1
    local canonical_index=$actual/.git/index
    local index_lock=$actual/.git/index.lock
    local expected_stage=$work_dir/expected-index.stage
    local actual_stage=$work_dir/actual-index.stage
    local index_flags=$work_dir/actual-index.flags
    local indexed_entry

    if [[ -e "$index_lock" || -L "$index_lock" ]]; then
        echo "error: refusing existing Test262 index lock: $index_lock" >&2
        return 1
    fi
    if [[ -e "$canonical_index" || -L "$canonical_index" ]]; then
        if [[ ! -f "$canonical_index" || -L "$canonical_index" ]]; then
            echo "error: refusing unsafe Test262 index: $canonical_index" >&2
            return 1
        fi
    fi

    index_tmp=$actual/.git/.index.$lock_token.tmp
    if [[ -e "$index_tmp" || -L "$index_tmp" ]]; then
        echo "error: refusing occupied Test262 temporary index path: $index_tmp" >&2
        return 1
    fi
    trusted_git --controlled-index "$index_tmp" \
        --git-dir="$actual/.git" --work-tree="$actual" read-tree "$expected_commit"
    if [[ ! -f "$index_tmp" || -L "$index_tmp" ]]; then
        echo "error: failed to create a regular canonical Test262 index" >&2
        return 1
    fi

    trusted_git --git-dir="$actual/.git" --work-tree="$actual" \
        ls-tree -r -z --full-tree \
        --format='%(objectmode) %(objectname) 0%x09%(path)' \
        "$expected_commit" > "$expected_stage"
    trusted_git --controlled-index "$index_tmp" \
        --git-dir="$actual/.git" --work-tree="$actual" \
        ls-files --stage -z > "$actual_stage"
    if ! cmp -s -- "$expected_stage" "$actual_stage"; then
        echo "error: generated Test262 index does not match the pinned commit" >&2
        return 1
    fi
    trusted_git --controlled-index "$index_tmp" \
        --git-dir="$actual/.git" --work-tree="$actual" \
        ls-files -v -z > "$index_flags"
    while IFS= read -r -d '' indexed_entry; do
        if [[ ${indexed_entry:0:1} != H ]]; then
            echo "error: generated Test262 index contains unsafe entry flags" >&2
            return 1
        fi
    done < "$index_flags"

    mv -f -- "$index_tmp" "$canonical_index"
    index_tmp=
    if [[ ! -f "$canonical_index" || -L "$canonical_index" ]]; then
        echo "error: published Test262 index is not a regular file" >&2
        return 1
    fi
}

authenticate_suite() {
    local actual=$1
    local expected_tree=$work_dir/expected-tree
    local expected_archive=$work_dir/expected.tar
    local diff_output=$work_dir/tree.diff
    local rel expected_member actual_member actual_commit member shallow_line
    local tree_entry_count archive_entry_count replacement

    if [[ ! -d "$actual" || -L "$actual" || ! -d "$actual/.git" || -L "$actual/.git" ]]; then
        echo "error: Test262 path is not a regular pinned git checkout: $actual" >&2
        return 1
    fi
    for unsafe_metadata in "$actual/.git/info/grafts" \
        "$actual/.git/info/attributes" \
        "$actual/.git/objects/info/alternates" \
        "$actual/.git/commondir"; do
        if [[ -e "$unsafe_metadata" || -L "$unsafe_metadata" ]]; then
            echo "error: refusing unsafe Test262 git metadata: $unsafe_metadata" >&2
            return 1
        fi
    done
    if [[ -L "$actual/.git/objects" || ! -d "$actual/.git/objects" ]]; then
        echo "error: refusing unsafe Test262 object directory" >&2
        return 1
    fi
    for metadata_dir in "$actual/.git/info" "$actual/.git/objects/info" "$actual/.git/refs"; do
        if [[ ! -d "$metadata_dir" || -L "$metadata_dir" ]]; then
            echo "error: refusing unsafe Test262 git metadata directory: $metadata_dir" >&2
            return 1
        fi
    done
    if [[ -e "$actual/.git/refs/replace" || -L "$actual/.git/refs/replace" ]]; then
        if [[ ! -d "$actual/.git/refs/replace" || -L "$actual/.git/refs/replace" ]]; then
            echo "error: refusing unsafe Test262 replacement refs directory" >&2
            return 1
        fi
    fi
    if [[ -e "$actual/.git/packed-refs" || -L "$actual/.git/packed-refs" ]]; then
        if [[ ! -f "$actual/.git/packed-refs" || -L "$actual/.git/packed-refs" ]]; then
            echo "error: refusing unsafe Test262 packed refs" >&2
            return 1
        fi
    fi
    replacement=$(find "$actual/.git/refs/replace" -mindepth 1 -print -quit 2>/dev/null || true)
    if [[ -n "$replacement" ]] || \
       { [[ -f "$actual/.git/packed-refs" ]] && \
         grep -E ' refs/replace/' "$actual/.git/packed-refs" >/dev/null; }; then
        echo "error: refusing Test262 replacement refs" >&2
        return 1
    fi
    if [[ -e "$actual/.git/shallow" || -L "$actual/.git/shallow" ]]; then
        if [[ ! -f "$actual/.git/shallow" || -L "$actual/.git/shallow" ]]; then
            echo "error: refusing unsafe Test262 shallow metadata" >&2
            return 1
        fi
        while IFS= read -r shallow_line; do
            if [[ ${#shallow_line} -ne 40 || "$shallow_line" == *[!0-9a-f]* ]]; then
                echo "error: refusing malformed Test262 shallow metadata" >&2
                return 1
            fi
        done < "$actual/.git/shallow"
    fi
    actual_commit=$(trusted_git --git-dir="$actual/.git" --work-tree="$actual" \
        rev-parse --verify 'HEAD^{commit}')
    if [[ "$actual_commit" != "$expected_commit" ]]; then
        echo "error: Test262 checkout is not at the pinned commit" >&2
        echo "expected: $expected_commit" >&2
        echo "actual:   $actual_commit" >&2
        return 1
    fi
    trusted_git --git-dir="$actual/.git" --work-tree="$actual" \
        cat-file -e "$expected_commit^{commit}"
    trusted_git --git-dir="$actual/.git" --work-tree="$actual" \
        fsck --strict --no-reflogs "$expected_commit" >/dev/null

    rm -rf -- "$expected_tree"
    mkdir -- "$expected_tree"
    trusted_git --git-dir="$actual/.git" --work-tree="$actual" \
        archive --format=tar --output="$expected_archive" "$expected_commit"
    tree_entry_count=$(trusted_git --git-dir="$actual/.git" --work-tree="$actual" \
        ls-tree -r -z --full-tree --name-only \
        "$expected_commit" | LC_ALL=C tr -cd '\000' | wc -c | awk '{print $1}')
    tar -xf "$expected_archive" -C "$expected_tree"
    archive_entry_count=$(find "$expected_tree" \( -type f -o -type l \) -print0 \
        | LC_ALL=C tr -cd '\000' | wc -c | awk '{print $1}')
    if [[ "$archive_entry_count" != "$tree_entry_count" ]]; then
        echo "error: Test262 commit archive omitted or changed tree members" >&2
        return 1
    fi
    (CDPATH='' cd -- "$expected_tree" && trusted_git apply --no-index --whitespace=nowarn "$patch")

    while IFS= read -r -d '' member; do
        rel=${member#"$expected_tree"/}
        if [[ "$rel" == .git || "$rel" == */.git || "$rel" == .git/* || "$rel" == */.git/* ]]; then
            echo "error: pinned Test262 tree contains a nested .git path: $rel" >&2
            return 1
        fi
        if [[ -L "$member" || ( ! -d "$member" && ! -f "$member" ) ]]; then
            echo "error: pinned Test262 tree contains a symlink or special member: $member" >&2
            return 1
        fi
    done < <(find "$expected_tree" -mindepth 1 -print0)
    while IFS= read -r -d '' member; do
        rel=${member#"$actual"/}
        if [[ "$rel" == .git || "$rel" == */.git || "$rel" == .git/* || "$rel" == */.git/* ]]; then
            echo "error: Test262 worktree contains a nested .git path: $rel" >&2
            return 1
        fi
        if [[ -L "$member" || ( ! -d "$member" && ! -f "$member" ) ]]; then
            echo "error: Test262 worktree contains a symlink or special member: $member" >&2
            return 1
        fi
    done < <(find "$actual" -mindepth 1 \( -path "$actual/.git" -prune -o -print0 \))

    if ! env -i "PATH=$PATH" DIFF_OPTIONS= LC_ALL=C \
        diff -qr -x .git -- "$expected_tree" "$actual" >"$diff_output"; then
        echo "error: Test262 worktree paths or bytes differ from pinned patched tree" >&2
        sed -n '1,20p' "$diff_output" >&2
        return 1
    fi

    while IFS= read -r -d '' expected_member; do
        [[ -f "$expected_member" ]] || continue
        rel=${expected_member#"$expected_tree"/}
        actual_member=$actual/$rel
        if { [[ -x "$expected_member" ]] && [[ ! -x "$actual_member" ]]; } || \
           { [[ ! -x "$expected_member" ]] && [[ -x "$actual_member" ]]; }; then
            echo "error: Test262 worktree executable mode differs from pinned tree: $rel" >&2
            return 1
        fi
    done < <(find "$expected_tree" -type f -print0)
}

cold_suite=0
suite_candidate=$suite
if [[ ! -e "$suite" && ! -L "$suite" ]]; then
    cold_suite=1
    suite_candidate=$work_dir/test262
    trusted_git init -q "$suite_candidate"
    trusted_git -C "$suite_candidate" remote add origin "$test262_url"
    trusted_git -C "$suite_candidate" fetch --depth=1 origin "$expected_commit" >&2
    trusted_git -C "$suite_candidate" checkout -q --detach "$expected_commit"
    trusted_git -C "$suite_candidate" apply --whitespace=nowarn "$patch"
fi

validate_canonical_config_target "$suite_candidate"
publish_canonical_config "$suite_candidate"
publish_canonical_exclude "$suite_candidate"
authenticate_suite "$suite_candidate"
publish_canonical_index "$suite_candidate"

oracles_output=$("$script_dir/build-quickjs-oracle.sh" --test262-oracles)
if [[ -z "$oracles_output" || "$oracles_output" == *$'\n'* ]]; then
    echo "error: build-quickjs-oracle.sh --test262-oracles returned an unexpected path" >&2
    exit 1
fi
if [[ "$oracles_output" != /* ]]; then
    oracles_output=$(CDPATH='' cd -- "$(dirname -- "$oracles_output")" && pwd)/$(basename -- "$oracles_output")
fi
if [[ "$oracles_output" != "$source_dir" ]]; then
    echo "error: build-quickjs-oracle.sh --test262-oracles returned an unexpected path" >&2
    exit 1
fi

if [[ $cold_suite -eq 1 ]]; then
    mv -- "$suite_candidate" "$suite"
fi

CDPATH='' cd -- "$suite"
pwd -P

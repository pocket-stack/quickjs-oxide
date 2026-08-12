#!/usr/bin/env bash
# Build the pinned upstream QuickJS release as a test-only differential oracle.

set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
version=2026-06-04
url=https://bellard.org/quickjs/quickjs-${version}.tar.xz
expected_sha256=b376e839b322978313d929fd20663b11ba58b75df5a46c126dd19ea2fa70ad2a
cache=${QJS_ORACLE_CACHE:-"$root/target/oracle"}
archive=$cache/quickjs-${version}.tar.xz
source_dir=$cache/quickjs-${version}
oracle=$source_dir/qjs
lock_dir=$cache/.quickjs-${version}.lock
lock_wait_seconds=${QJS_ORACLE_LOCK_WAIT_SECONDS:-30}
source_only=0
test262_oracles=0

usage() {
    echo "usage: $0 [--source-only|--test262-oracles]" >&2
}

case ${1-} in
    "") ;;
    --source-only) source_only=1 ;;
    --test262-oracles) test262_oracles=1 ;;
    *) usage; exit 2 ;;
esac
if [[ $# -gt 1 ]]; then
    usage
    exit 2
fi

case $lock_wait_seconds in
    ""|*[!0-9]*)
        echo "error: QJS_ORACLE_LOCK_WAIT_SECONDS must be between 0 and 3600 seconds" >&2
        exit 2
        ;;
esac
while [[ $lock_wait_seconds == 0?* ]]; do
    lock_wait_seconds=${lock_wait_seconds#0}
done
case $lock_wait_seconds in
    [0-9]|[0-9][0-9]|[0-9][0-9][0-9]|[12][0-9][0-9][0-9]|3[0-5][0-9][0-9]|3600) ;;
    *)
        echo "error: QJS_ORACLE_LOCK_WAIT_SECONDS must be between 0 and 3600 seconds" >&2
        exit 2
        ;;
esac
lock_wait_seconds=$((10#$lock_wait_seconds))

command -v tar >/dev/null 2>&1 || {
    echo "error: tar is required to extract the QuickJS oracle" >&2
    exit 2
}
command -v cmp >/dev/null 2>&1 || {
    echo "error: cmp is required to authenticate the QuickJS oracle source" >&2
    exit 2
}

mkdir -p -- "$cache"

lock_token="$$.$(date +%s).${RANDOM-0}"
lock_owned=0
work_dir=$cache/.quickjs-${version}.work.$lock_token
archive_tmp=$cache/.quickjs-${version}.archive.$lock_token.tmp
publish_tmp=$source_dir/.qjs.$lock_token.tmp
runner_publish_tmp=$source_dir/.run-test262.$lock_token.tmp
library_publish_tmp=$source_dir/.libquickjs.a.$lock_token.tmp
stage_source=

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
    rm -f -- "$archive_tmp" "$publish_tmp" "$runner_publish_tmp" \
        "$library_publish_tmp" 2>/dev/null || true
    rm -rf -- "$work_dir" 2>/dev/null || true
    release_lock
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

lock_deadline=$(( $(date +%s) + lock_wait_seconds ))
while ! mkdir -- "$lock_dir" 2>/dev/null; do
    if [[ ! -d "$lock_dir" || -L "$lock_dir" ]]; then
        echo "error: oracle lock path is not a directory: $lock_dir" >&2
        exit 1
    fi

    if [[ -e "$lock_dir/owner" || -L "$lock_dir/owner" ]]; then
        if [[ ! -f "$lock_dir/owner" || -L "$lock_dir/owner" ]]; then
            echo "error: refusing unsafe oracle lock owner: $lock_dir/owner" >&2
            exit 1
        fi
        owner_line=$(sed -n '1p' "$lock_dir/owner" 2>/dev/null || true)
        owner_pid=${owner_line#* }
        owner_token=${owner_line%% *}
        case $owner_pid in
            ""|*[!0-9]*)
                echo "error: refusing malformed or stale oracle lock: $lock_dir" >&2
                exit 1
                ;;
        esac
        if [[ -z "$owner_token" || "$owner_token" == "$owner_line" ]] || \
           ! kill -0 "$owner_pid" 2>/dev/null; then
            echo "error: refusing stale oracle lock: $lock_dir" >&2
            exit 1
        fi
    fi

    if [[ $(date +%s) -ge $lock_deadline ]]; then
        echo "error: timed out waiting for active oracle lock: $lock_dir" >&2
        exit 1
    fi
    sleep 1
done

umask 077
printf '%s %s\n' "$lock_token" "$$" > "$lock_dir/owner"
lock_owned=1
mkdir -- "$work_dir"

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required to verify the oracle" >&2
        exit 2
    fi
}

verify_archive() {
    local candidate=$1
    local actual_sha256

    if [[ ! -f "$candidate" || -L "$candidate" ]]; then
        echo "error: QuickJS oracle archive is not a regular file: $candidate" >&2
        return 1
    fi
    actual_sha256=$(sha256_file "$candidate")
    if [[ "$actual_sha256" != "$expected_sha256" ]]; then
        echo "error: QuickJS oracle archive checksum mismatch" >&2
        echo "expected: $expected_sha256" >&2
        echo "actual:   $actual_sha256" >&2
        return 1
    fi
}

if [[ ! -e "$archive" && ! -L "$archive" ]]; then
    command -v curl >/dev/null 2>&1 || {
        echo "error: curl is required to download the QuickJS oracle" >&2
        exit 2
    }
    curl -fL "$url" -o "$archive_tmp"
    verify_archive "$archive_tmp"
    mv -f -- "$archive_tmp" "$archive"
fi
verify_archive "$archive"

stage_parent=$work_dir/stage
mkdir -- "$stage_parent"
tar -xJf "$archive" -C "$stage_parent"
stage_source=$stage_parent/quickjs-${version}
if [[ ! -d "$stage_source" || -L "$stage_source" ]]; then
    echo "error: verified QuickJS archive did not extract the expected source tree" >&2
    exit 1
fi

authenticate_source() {
    local expected=$1
    local actual=$2
    local manifest=$work_dir/archive-members
    local rel expected_member actual_member expected_link actual_link

    if [[ ! -d "$actual" || -L "$actual" ]]; then
        echo "error: cached QuickJS source is not a directory: $actual" >&2
        return 1
    fi

    (CDPATH='' cd -- "$expected" && find . -mindepth 1 -print) > "$manifest"
    while IFS= read -r rel; do
        expected_member=$expected/${rel#./}
        actual_member=$actual/${rel#./}
        if [[ -L "$expected_member" ]]; then
            if [[ ! -L "$actual_member" ]]; then
                echo "error: cached QuickJS source member has wrong type: $actual_member" >&2
                return 1
            fi
            expected_link=$(readlink "$expected_member")
            actual_link=$(readlink "$actual_member")
            if [[ "$actual_link" != "$expected_link" ]]; then
                echo "error: cached QuickJS source symlink differs from verified archive: $actual_member" >&2
                return 1
            fi
        elif [[ -d "$expected_member" ]]; then
            if [[ ! -d "$actual_member" || -L "$actual_member" ]]; then
                echo "error: cached QuickJS source member has wrong type: $actual_member" >&2
                return 1
            fi
        elif [[ -f "$expected_member" ]]; then
            if [[ ! -f "$actual_member" || -L "$actual_member" ]]; then
                echo "error: cached QuickJS source member has wrong type: $actual_member" >&2
                return 1
            fi
            if ! cmp -s -- "$expected_member" "$actual_member"; then
                echo "error: cached QuickJS source differs from verified archive: $actual_member" >&2
                return 1
            fi
        else
            echo "error: verified QuickJS archive contains an unsupported member type: $expected_member" >&2
            return 1
        fi
    done < "$manifest"
}

source_exists=0
if [[ -e "$source_dir" || -L "$source_dir" ]]; then
    source_exists=1
    authenticate_source "$stage_source" "$source_dir"
fi

validate_publish_destination() {
    local destination=$1
    local label=$2
    if [[ -e "$destination" || -L "$destination" ]]; then
        if [[ ! -f "$destination" || -L "$destination" ]]; then
            echo "error: refusing unsafe cached QuickJS $label destination: $destination" >&2
            return 1
        fi
    fi
}

if [[ $source_only -eq 1 ]]; then
    if [[ $source_exists -eq 0 ]]; then
        mv -- "$stage_source" "$source_dir"
    fi
    printf '%s\n' "$source_dir"
    exit 0
fi

if [[ $source_exists -eq 1 ]]; then
    validate_publish_destination "$oracle" qjs
    if [[ $test262_oracles -eq 1 ]]; then
        validate_publish_destination "$source_dir/run-test262" run-test262
        validate_publish_destination "$source_dir/libquickjs.a" libquickjs.a
    fi
fi

if [[ $test262_oracles -eq 1 ]]; then
    "${MAKE:-make}" -C "$stage_source" qjs run-test262 libquickjs.a >&2
else
    "${MAKE:-make}" -C "$stage_source" qjs >&2
fi
staged_oracle=$stage_source/qjs
if [[ ! -f "$staged_oracle" || -L "$staged_oracle" || ! -x "$staged_oracle" ]]; then
    echo "error: QuickJS build did not produce an executable regular qjs" >&2
    exit 1
fi

if [[ $test262_oracles -eq 1 ]]; then
    staged_runner=$stage_source/run-test262
    staged_library=$stage_source/libquickjs.a
    staged_obj=$stage_source/.obj
    if [[ ! -f "$staged_runner" || -L "$staged_runner" || ! -x "$staged_runner" ]]; then
        echo "error: QuickJS build did not produce an executable regular run-test262" >&2
        exit 1
    fi
    if [[ ! -f "$staged_library" || -L "$staged_library" ]]; then
        echo "error: QuickJS build did not produce a regular libquickjs.a" >&2
        exit 1
    fi
    if [[ ! -d "$staged_obj" || -L "$staged_obj" ]]; then
        echo "error: QuickJS build did not produce a regular .obj directory" >&2
        exit 1
    fi
    while IFS= read -r -d '' obj_member; do
        if [[ -L "$obj_member" || ( ! -d "$obj_member" && ! -f "$obj_member" ) ]]; then
            echo "error: fresh QuickJS .obj contains an unsafe member: $obj_member" >&2
            exit 1
        fi
    done < <(find "$staged_obj" -mindepth 1 -print0)
fi

verify_parent=$work_dir/verify
mkdir -- "$verify_parent"
tar -xJf "$archive" -C "$verify_parent"
verify_source=$verify_parent/quickjs-${version}
if [[ ! -d "$verify_source" || -L "$verify_source" ]]; then
    echo "error: verified QuickJS archive did not reproduce the source tree" >&2
    exit 1
fi
authenticate_source "$verify_source" "$stage_source"

if [[ $source_exists -eq 0 ]]; then
    mv -- "$stage_source" "$source_dir"
else
    cp -p -- "$staged_oracle" "$publish_tmp"
    if [[ ! -f "$publish_tmp" || -L "$publish_tmp" || ! -x "$publish_tmp" ]]; then
        echo "error: failed to stage the QuickJS oracle executable" >&2
        exit 1
    fi
    if [[ $test262_oracles -eq 1 ]]; then
        cp -p -- "$staged_runner" "$runner_publish_tmp"
        if [[ ! -f "$runner_publish_tmp" || -L "$runner_publish_tmp" || ! -x "$runner_publish_tmp" ]]; then
            echo "error: failed to stage the QuickJS run-test262 executable" >&2
            exit 1
        fi
        cp -p -- "$staged_library" "$library_publish_tmp"
        if [[ ! -f "$library_publish_tmp" || -L "$library_publish_tmp" ]]; then
            echo "error: failed to stage the QuickJS libquickjs.a" >&2
            exit 1
        fi
    fi
    mv -f -- "$publish_tmp" "$oracle"
    if [[ ! -f "$oracle" || -L "$oracle" || ! -x "$oracle" ]]; then
        echo "error: published QuickJS qjs is not an executable regular file" >&2
        exit 1
    fi
    if [[ $test262_oracles -eq 1 ]]; then
        mv -f -- "$runner_publish_tmp" "$source_dir/run-test262"
        if [[ ! -f "$source_dir/run-test262" || -L "$source_dir/run-test262" || \
              ! -x "$source_dir/run-test262" ]]; then
            echo "error: published QuickJS run-test262 is not an executable regular file" >&2
            exit 1
        fi
        mv -f -- "$library_publish_tmp" "$source_dir/libquickjs.a"
        if [[ ! -f "$source_dir/libquickjs.a" || \
              -L "$source_dir/libquickjs.a" ]]; then
            echo "error: published QuickJS libquickjs.a is not a regular file" >&2
            exit 1
        fi
    fi
fi

if [[ $test262_oracles -eq 1 ]]; then
    printf '%s\n' "$source_dir"
else
    printf '%s\n' "$oracle"
fi

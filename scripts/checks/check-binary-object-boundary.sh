#!/usr/bin/env bash
# Keep the release-pinned binary-object archive codec isolated from the
# compiler, VM, runtime publication path, and public crate surface.

set -euo pipefail
export LC_ALL=C

tool_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
script_dir=$(CDPATH= cd -- "$tool_dir/.." && pwd)
boundary_dir="$script_dir/checks/binary_object"
repository_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

die() {
    echo "error: $*" >&2
    exit 1
}

command -v python3 >/dev/null 2>&1 || die "python3 is required"

scan_root() {
    local candidate_root=$1
    local self_test_token=${2-${QUICKJS_OXIDE_BOUNDARY_SELF_TEST_TOKEN-}}
    local root

    root=$(CDPATH='' cd -- "$candidate_root" && pwd)
    PYTHONPATH="$script_dir/checks${PYTHONPATH:+:$PYTHONPATH}" \
        python3 -m binary_object "$root" "$self_test_token"
}

source "$boundary_dir/canaries/receipts.sh"

case ${1:-} in
    "") ;;
    --scan-only)
        [[ $# == 2 ]] || die "usage: $0 --scan-only ROOT"
        scan_root "$2"
        exit 0
        ;;
    --stage3i-receipt-canaries)
        [[ $# == 1 ]] \
            || die "usage: $0 --stage3i-receipt-canaries"
        scan_root "$repository_root"
        receipt_canary_tmp=$(mktemp -d \
            "${TMPDIR:-/tmp}/quickjs-oxide-stage3i-receipts.XXXXXX")
        trap 'rm -rf -- "$receipt_canary_tmp"' EXIT HUP INT TERM
        run_stage3i_receipt_escape_canaries "$receipt_canary_tmp"
        echo "Stage3I receipt escape canaries passed: 12/12 rejected"
        exit 0
        ;;
    *) die "usage: $0 [--scan-only ROOT|--stage3i-receipt-canaries]" ;;
esac

scan_root "$repository_root"

python3 "$boundary_dir/canaries/check_integration.py" "$repository_root"

source "$boundary_dir/canaries/fixture.sh"

source "$boundary_dir/canaries/jobs.sh"
source "$boundary_dir/canaries/helpers.sh"

source "$boundary_dir/canaries/surface.sh"
source "$boundary_dir/canaries/ordinary_leaf.sh"
source "$boundary_dir/canaries/translation.sh"
source "$boundary_dir/canaries/stage3d.sh"
source "$boundary_dir/canaries/stage3e.sh"
source "$boundary_dir/canaries/stage3f.sh"
source "$boundary_dir/canaries/stage3g.sh"
source "$boundary_dir/canaries/stage3h.sh"
source "$boundary_dir/canaries/stage3i.sh"
source "$boundary_dir/canaries/scalar.sh"
source "$boundary_dir/canaries/native_plan.sh"
source "$boundary_dir/canaries/shared_transport.sh"

finish_canaries

echo "binary-object production boundary passed; all isolation canaries were rejected"

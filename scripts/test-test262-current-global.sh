#!/usr/bin/env bash
# Stable entry point for the latest checksum-bound Test262 milestone.

set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
[[ $# -le 1 ]] \
    || { echo 'usage: test-test262-current-global.sh [--check|--full]' >&2; exit 2; }

case ${1-} in
    ''|--check)
        exec "$script_dir/test-test262-agent-broadcast-a-global.sh" --check
        ;;
    --full)
        exec "$script_dir/test-test262-agent-broadcast-a-global.sh" --full
        ;;
    -h|--help)
        printf 'usage: %s [--check|--full]\n' "${0##*/}"
        printf '  --check  authenticate the latest focused and canonical receipts\n'
        printf '  --full   rerun the complete 102037-variant canonical vector\n'
        ;;
    *)
        echo 'usage: test-test262-current-global.sh [--check|--full]' >&2
        exit 2
        ;;
esac

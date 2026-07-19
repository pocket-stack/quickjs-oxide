#!/usr/bin/env bash
set -euo pipefail
exec "$(dirname -- "$0")/test-test262-json-focused.sh" json-stringify JSON.stringify

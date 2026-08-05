#!/usr/bin/env bash
# Authenticate the R3dj implemented non-agent bounded Atomics.wait frontier.
#
# This gate proves the source-derived 33-path boundary, runtime host policy,
# native cross-runtime waiter core, and the complete Oxide/QuickJS 66-variant
# vectors. It deliberately does not claim Test262 agent or Atomics.waitAsync
# parity.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-atomics-wait-nonagent-bounded-baseline.txt
sab_ledger=tests/test262-shared-array-buffer-universe.tsv
atomics_ledger=tests/test262-atomics-universe.tsv
tagged=tests/test262-atomics-wait-nonagent-bounded-tagged.txt
spillover=tests/test262-atomics-wait-nonagent-bounded-spillover.txt
combined=tests/test262-atomics-wait-nonagent-bounded.txt
ledger=tests/test262-atomics-wait-nonagent-bounded.tsv
profile=tests/test262-atomics-wait-nonagent-bounded.conf
quickjs_receipt=tests/test262-atomics-wait-nonagent-bounded-quickjs-receipt.txt
canonical_baseline=tests/test262-full-baseline.txt
upstream=compat/upstream.toml
workers=${TEST262_WORKERS:-8}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  authenticate the implemented R3dj frontier\n'
    printf '  default  same as --check\n'
}

case ${1-} in
    '') ;;
    --check) ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { usage >&2; exit 2; }
[[ "$workers" =~ ^[1-9][0-9]*$ ]] \
    || { echo 'error: invalid Test262 worker count' >&2; exit 2; }
[[ -z ${TEST262_RUNNER+x} ]] \
    || { echo 'error: TEST262_RUNNER override is forbidden for R3dj' >&2; exit 2; }

die() { echo "error: $*" >&2; exit 1; }
sha() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        die 'sha256sum or shasum is required'
    fi
}
lines() { wc -l <"$1" | tr -d '[:space:]'; }
value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$baseline"
}
canonical_value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$canonical_baseline"
}
section() {
    awk -v wanted="[$2]" \
        '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' \
        "$1"
}
check_file() {
    [[ -f "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated input drifted: $1"
}
toml_test262_value() {
    awk -v wanted="$2" '
        $0=="[test262]"{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted)next
            answer=substr($0,separator+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$1"
}
toml_quickjs_value() {
    awk -v wanted="$2" '
        $0=="[quickjs]"{inside=1;next} /^\[/{inside=0}
        inside{
            separator=index($0,"=");if(!separator)next
            key=substr($0,1,separator-1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", key)
            if(key!=wanted)next
            answer=substr($0,separator+1)
            gsub(/^[[:space:]]+|[[:space:]]+$/, "", answer)
            if(answer~/^".*"$/)answer=substr(answer,2,length(answer)-2)
            print answer;found++
        }
        END{if(found!=1)exit 1}
    ' "$1"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
generated_value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$generated_summary"
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-r3dj.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
generated_summary=$tmp/summary.txt

check_file "$baseline" 137 \
    3eac048bda22dccd8b5fb299bab5d5a1c541518c0c4b60dbbc042a58617f443d
[[ "$(value canonical_baseline)" == "$canonical_baseline" ]] \
    || die 'R3dj canonical baseline path drifted'
check_file "$canonical_baseline" "$(value canonical_baseline_lines)" \
    "$(value canonical_baseline_sha256)"
[[ "$(canonical_value variants)" == "$(value canonical_variants)" \
    && "$(canonical_value runnable)" == "$(value canonical_runnable)" \
    && "$(canonical_value passes)" == "$(value canonical_passes)" \
    && "$(canonical_value tsv_sha256)" == "$(value canonical_tsv_sha256)" \
    && "$(canonical_value jsonl_sha256)" == "$(value canonical_jsonl_sha256)" ]] \
    || die 'R3dj canonical full-vector bridge drifted'
check_file "$sab_ledger" "$(value shared_array_buffer_universe_lines)" \
    "$(value shared_array_buffer_universe_sha256)"
check_file "$atomics_ledger" "$(value atomics_universe_lines)" \
    "$(value atomics_universe_sha256)"
check_file "$tagged" "$(value tagged_paths)" "$(value tagged_manifest_sha256)"
check_file "$spillover" "$(value spillover_paths)" "$(value spillover_manifest_sha256)"
check_file "$combined" "$(value combined_paths)" "$(value combined_manifest_sha256)"
check_file "$ledger" "$(value ledger_lines)" "$(value ledger_sha256)"
check_file "$profile" "$(value scoped_profile_lines)" "$(value scoped_profile_sha256)"
check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
    "$(value quickjs_receipt_sha256)"

for manifest in "$tagged" "$spillover" "$combined"; do
    sort -c "$manifest" || die "manifest is not bytewise sorted: $manifest"
    [[ -z "$(sort "$manifest" | uniq -d)" ]] \
        || die "manifest contains duplicates: $manifest"
done
[[ -z "$(comm -12 "$tagged" "$spillover")" ]] \
    || die 'tagged and spillover manifests overlap'
sort -u "$tagged" "$spillover" >"$tmp/static-combined.txt"
diff -u "$combined" "$tmp/static-combined.txt" \
    || die 'combined manifest is not the exact tagged/spillover union'

[[ "$(value milestone_kind)" == bounded-implementation \
    && "$(value oxide_focused_report)" == authenticated-pass \
    && "$(value scope_semantics)" == bounded-nonagent-synchronous-wait \
    && "$(value waiter_core)" == native-cross-runtime \
    && "$(value waiter_parity)" == bounded-only \
    && "$(value implementation_runtime_tree)" == changed-from-base \
    && "$(value runner_source)" == current-worktree \
    && "$(value runner_override)" == forbidden \
    && "$(value agent_paths)" == excluded \
    && "$(value wait_async_paths)" == excluded \
    && "$(value selection_only)" == false ]] \
    || die 'R3dj implementation boundary drifted'

implementation_base_commit=$(value implementation_base_commit)
[[ "$(git rev-parse --verify "$implementation_base_commit^{commit}" 2>/dev/null)" \
        == "$implementation_base_commit" ]] \
    || die 'R3dj implementation base commit is unavailable'
if git diff --quiet "$implementation_base_commit" -- src/runtime.rs src/runtime; then
    die 'R3dj implementation contains no runtime changes from its selected boundary'
fi

scanner_source=$(value code_token_scanner)
scanner_excerpt=$tmp/js-code-tokens.pl
awk '/^sub js_code_tokens \{$/{inside=1} /^sub host_tokens \{$/{inside=0} inside{print}' \
    "$scanner_source" >"$scanner_excerpt"
check_file "$scanner_excerpt" "$(value code_token_scanner_lines)" \
    "$(value code_token_scanner_sha256)"

[[ "$(toml_quickjs_value "$upstream" version)" == "$(value quickjs)" \
    && "$(toml_quickjs_value "$upstream" source_sha256)" \
        == "$(value quickjs_source_sha256)" \
    && "$(toml_test262_value "$upstream" repository)" \
        == https://github.com/tc39/test262.git \
    && "$(toml_test262_value "$upstream" commit)" == "$(value test262)" \
    && "$(toml_test262_value "$upstream" patch_sha256)" \
        == "$(value test262_patch_sha256)" \
    && "$(toml_test262_value "$upstream" config_sha256)" \
        == "$(value test262_config_sha256)" \
    && "$(toml_test262_value "$upstream" test_count)" \
        == "$(value test262_metadata_records)" \
    && "$(toml_test262_value "$upstream" metadata_records_sha256)" \
        == "$(value test262_metadata_sha256)" ]] \
    || die 'compat/upstream.toml identity drifted'

[[ "$(section "$profile" features | wc -l | tr -d '[:space:]')" \
        == "$(value scoped_profile_features)" \
    && "$(section "$profile" features | sha /dev/stdin)" \
        == "$(value scoped_profile_features_sha256)" \
    && -z "$(section "$profile" audited-negative-tests)" \
    && -z "$(section "$profile" execution)" ]] \
    || die 'bounded implementation profile identity drifted'

gate_target=$tmp/cargo-target
CARGO_TARGET_DIR="$gate_target" cargo test --locked --quiet --lib \
    runtime::intrinsics::atomics -- --test-threads=1
CARGO_TARGET_DIR="$gate_target" cargo test --locked --quiet --lib \
    can_block_policy_is_runtime_wide_and_isolated
CARGO_TARGET_DIR="$gate_target" cargo test --locked --quiet --bin run-test262 \
    atomics_wait_nonagent_bounded
CARGO_TARGET_DIR="$gate_target" cargo build --locked --release --quiet --bin run-test262
runner=$gate_target/release/run-test262
[[ -x "$runner" ]] || die 'current-worktree Test262 runner is unavailable'

suite=$("$script_dir/prepare-test262.sh")
source_dir=$(dirname -- "$suite")
expected_status=$' M harness/atomicsHelper.js\n M harness/regExpUtils.js'
actual_status=$(git -C "$suite" status --porcelain=v1 --untracked-files=all | sort)
[[ "$(basename -- "$source_dir")" == "quickjs-$(value quickjs)" \
    && "$(git -C "$suite" rev-parse --verify 'HEAD^{commit}')" == "$(value test262)" \
    && "$(sha "$source_dir/tests/test262.patch")" == "$(value test262_patch_sha256)" \
    && "$(sha "$source_dir/test262.conf")" == "$(value test262_config_sha256)" \
    && "$(sha "$suite/harness/atomicsHelper.js")" \
        == "$(value patched_atomics_helper_sha256)" \
    && "$(sha "$suite/harness/regExpUtils.js")" \
        == "$(value patched_regexp_utils_sha256)" \
    && "$actual_status" == "$expected_status" ]] \
    || die 'prepared QuickJS/Test262 identity drifted'
git -C "$suite" apply --reverse --check "$source_dir/tests/test262.patch" \
    || die 'prepared Test262 patch is not reverse-applicable'
[[ "$(grep -Fxc Atomics "$source_dir/test262.conf")" == 1 \
    && "$(grep -Fxc SharedArrayBuffer "$source_dir/test262.conf")" == 1 \
    && "$(grep -Fxc 'Atomics.waitAsync=skip' "$source_dir/test262.conf")" == 1 ]] \
    || die 'pinned QuickJS Atomics.wait boundary drifted'

{
    printf '%s\n' \
        'use strict;' \
        'use warnings;' \
        'use Digest::SHA qw(sha256_hex);' \
        'use File::Find;'
    sed -n '1,$p' "$scanner_excerpt"
    sed -n '/^__R3DJ_PERL__$/,/^__R3DJ_PERL_END__$/p' "$0" | sed '1d;$d'
} | perl - "$suite" "$sab_ledger" "$atomics_ledger" "$tmp"
# The remainder of this file after __R3DJ_PERL__ is Perl source consumed above.
# Bash resumes from the explicit exit marker emitted before that source.
exit_marker=$tmp/perl-complete
[[ -f "$exit_marker" ]] || die 'R3dj source scanner did not complete'

diff -u "$tagged" "$tmp/tagged.txt" \
    || die 'R3dj tagged projection drifted'
diff -u "$spillover" "$tmp/spillover.txt" \
    || die 'R3dj spillover projection drifted'
diff -u "$combined" "$tmp/combined.txt" \
    || die 'R3dj combined source closure drifted'
diff -u "$ledger" "$tmp/ledger.tsv" \
    || die 'R3dj source-derived ledger drifted'
diff -u <(section "$profile" features) "$tmp/combined.features" \
    || die 'R3dj profile is not the exact feature union'

[[ "$(lines "$tmp/all-js.txt")" == "$(value suite_js_paths)" \
    && "$(sha "$tmp/all-js.txt")" == "$(value suite_js_paths_sha256)" \
    && "$(lines "$tmp/raw.txt")" == "$(value raw_wait_member_paths)" \
    && "$(sha "$tmp/raw.txt")" == "$(value raw_wait_member_paths_sha256)" \
    && "$(sha "$tmp/raw-sources.tsv")" \
        == "$(value raw_wait_source_projection_sha256)" \
    && "$(sha "$tmp/tagged-sources.tsv")" \
        == "$(value tagged_source_projection_sha256)" \
    && "$(sha "$tmp/tagged-keys.tsv")" == "$(value tagged_keys_sha256)" \
    && "$(sha "$tmp/tagged.features")" == "$(value tagged_features_sha256)" \
    && "$(sha "$tmp/spillover-sources.tsv")" \
        == "$(value spillover_source_projection_sha256)" \
    && "$(sha "$tmp/spillover-keys.tsv")" == "$(value spillover_keys_sha256)" \
    && "$(sha "$tmp/spillover.features")" == "$(value spillover_features_sha256)" \
    && "$(sha "$tmp/combined-sources.tsv")" \
        == "$(value combined_source_projection_sha256)" \
    && "$(sha "$tmp/combined-keys.tsv")" == "$(value combined_keys_sha256)" \
    && "$(sha "$tmp/combined.features")" == "$(value combined_features_sha256)" \
    && "$(sha "$tmp/noncall.txt")" == "$(value code_non_call_member_paths_sha256)" \
    && "$(sha "$tmp/misfiled.txt")" == "$(value misfiled_paths_sha256)" \
    && "$(sha "$tmp/finite.txt")" == "$(value finite_timeout_paths_sha256)" \
    && "$(sha "$tmp/abrupt.txt")" == "$(value pre_wait_abrupt_paths_sha256)" ]] \
    || die 'R3dj source, variant, or semantic projection drifted'

for key in \
    suite_js_paths raw_wait_member_paths raw_shared_agent_paths \
    raw_non_shared_paths raw_shared_can_block_false_paths \
    raw_shared_no_extra_host_paths code_wait_member_paths \
    code_direct_wait_paths code_direct_wait_calls code_non_call_member_paths \
    raw_only_paths wait_async_raw_paths wait_async_intersection_paths \
    bracket_wait_paths optional_wait_paths aliased_wait_paths metadata_missing_paths \
    tagged_paths tagged_variants tagged_features spillover_paths \
    spillover_variants spillover_features combined_paths combined_variants \
    combined_features misfiled_paths finite_timeout_paths finite_timeout_calls \
    pre_wait_abrupt_paths infinite_wait_paths not_equal_paths notify_wakeup_paths \
    host_can_block_true_paths host_can_block_false_paths \
    host_detach_array_buffer_paths host_none_paths; do
    actual=$(generated_value "$key")
    expected=$(value "$key")
    [[ "$actual" == "$expected" ]] \
        || die "R3dj generated count drifted: $key (expected $expected, found $actual)"
done

rejection_log=$tmp/rejected.log
expect_rejected_selection() {
    local expected=$1
    shift
    if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" "$@" --report "$tmp/rejected.tsv" \
        --mode both >"$rejection_log" 2>&1; then
        die "bounded profile unexpectedly accepted an unpinned selection: $*"
    fi
    grep -Fq "$expected" "$rejection_log" \
        || { sed -n '1,120p' "$rejection_log" >&2; die "unexpected selection rejection: $*"; }
}
expect_rejected_selection 'requires its pinned manifest' --all
expect_rejected_selection 'requires its pinned manifest' \
    --test test/built-ins/Atomics/wait/false-for-timeout.js
expect_rejected_selection 'requires tests/test262-atomics-wait-nonagent-bounded.txt' \
    --manifest "$tagged"
expect_rejected_selection 'requires tests/test262-atomics-wait-nonagent-bounded.txt' \
    --manifest "$spillover"
expect_rejected_selection 'requires tests/test262-atomics-wait-nonagent-bounded.txt' \
    --manifest tests/test262-shared-array-buffer-core.txt

focused_report=$tmp/focused.tsv
focused_json=${focused_report%.tsv}.jsonl
focused_log=$tmp/focused.log
"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$combined" --report "$focused_report" \
    --mode both --timeout-ms 30000 --workers "$workers" \
    >"$focused_log" 2>&1 \
    || { sed -n '1,160p' "$focused_log" >&2; die 'exact R3dj selection failed'; }

[[ -f "$focused_report" && -f "$focused_json" \
    && "$(header "$focused_report" quickjs)" == "$(value quickjs)" \
    && "$(header "$focused_report" test262)" == "$(value test262)" \
    && "$(header "$focused_report" test262_patch_sha256)" \
        == "$(value test262_patch_sha256)" \
    && "$(header "$focused_report" test262_config_sha256)" \
        == "$(value test262_config_sha256)" \
    && "$(header "$focused_report" test262_metadata_sha256)" \
        == "$(value test262_metadata_sha256)" \
    && "$(header "$focused_report" oxide_profile_sha256)" \
        == "$(value scoped_profile_sha256)" \
    && "$(header "$focused_report" profile)" == test262-canonical-classified-v2 \
    && "$(header "$focused_report" mode)" == both \
    && "$(report_rows "$focused_report" | wc -l | tr -d '[:space:]')" \
        == "$(value oxide_focused_rows)" \
    && "$(report_keys "$focused_report" | sha /dev/stdin)" \
        == "$(value combined_keys_sha256)" \
    && "$(lines "$focused_report")" == "$(value oxide_focused_report_lines)" \
    && "$(lines "$focused_json")" == "$(value oxide_focused_jsonl_lines)" \
    && "$(sha "$focused_report")" == "$(value oxide_focused_tsv_sha256)" \
    && "$(sha "$focused_json")" == "$(value oxide_focused_jsonl_sha256)" \
    && "$(report_summary "$focused_report")" == "$(computed_summary "$focused_report")" \
    && "$(report_summary "$focused_report")" == "$(value oxide_focused_summary)" ]] \
    || die 'authenticated R3dj Oxide report identity drifted'

outcome_count() {
    report_rows "$focused_report" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{count++} END{print count+0}'
}
outcome_paths_sha() {
    report_rows "$focused_report" | awk -F'\t' -v wanted="$1" \
        '$7==wanted{print $1}' | sort -u | sha /dev/stdin
}
[[ "$(outcome_count pass)" == "$(value oxide_focused_passes)" \
    && "$(outcome_count fail-runtime)" == "$(value oxide_focused_fail_runtime)" \
    && "$(outcome_count unsupported-host-can-block-false)" \
        == "$(value oxide_focused_unsupported_host_can_block_false)" \
    && "$(outcome_paths_sha pass)" == "$(value oxide_focused_pass_paths_sha256)" \
    && "$(outcome_paths_sha fail-runtime)" \
        == "$(value oxide_focused_fail_paths_sha256)" \
    && "$(outcome_paths_sha unsupported-host-can-block-false)" \
        == "$(value oxide_focused_unsupported_paths_sha256)" \
    && "$(report_rows "$focused_report" | awk -F'\t' '$8=="selection"{count++} END{print count+0}')" \
        == "$(value oxide_focused_unsupported_host_can_block_false)" ]] \
    || die 'authenticated R3dj Oxide outcome vector drifted'

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$combined"
quickjs_log=$tmp/quickjs.log
if ! (cd "$source_dir" && ./run-test262 -m -c test262.conf -a \
        -T "$workers" -f "${files[@]}") >"$quickjs_log" 2>&1; then
    tail -n 100 "$quickjs_log" >&2
    die 'pinned QuickJS failed the R3dj frontier'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$quickjs_log" \
    || [[ "$(grep -Fc 'Average memory statistics for 66 tests:' "$quickjs_log")" != 1 ]]; then
    tail -n 100 "$quickjs_log" >&2
    die 'pinned QuickJS R3dj receipt drifted'
fi
{
    echo '# Pinned QuickJS oracle receipt for the R3dj non-agent bounded Atomics.wait frontier.'
    echo "quickjs=$(value quickjs)"
    echo "test262=$(value test262)"
    echo "tagged_manifest_sha256=$(value tagged_manifest_sha256)"
    echo "spillover_manifest_sha256=$(value spillover_manifest_sha256)"
    echo "combined_manifest_sha256=$(value combined_manifest_sha256)"
    echo "tagged_paths=$(value tagged_paths)"
    echo "tagged_variants=$(value tagged_variants)"
    echo "tagged_passes=$(value quickjs_tagged_passes)"
    echo "spillover_paths=$(value spillover_paths)"
    echo "spillover_variants=$(value spillover_variants)"
    echo "spillover_passes=$(value quickjs_spillover_passes)"
    echo "combined_paths=$(value combined_paths)"
    echo "combined_variants=$(value combined_variants)"
    echo "combined_passes=$(value quickjs_combined_passes)"
    echo "failed=$(value quickjs_failed)"
    echo "skipped_feature=$(value quickjs_skipped_feature)"
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
diff -u "$quickjs_receipt" "$tmp/quickjs-receipt.txt" \
    || die 'pinned QuickJS R3dj receipt projection drifted'

echo 'R3dj implementation verified: full-suite source closure 93 raw -> 33 bounded / 57 agent / 3 metadata-only; Oxide 66/66; pinned QuickJS 66/66; native cross-runtime waiter core passed.'
exit 0

: <<'__R3DJ_PERL_END__'
__R3DJ_PERL__
my ($suite, $sab_ledger, $atomics_ledger, $outdir) = @ARGV;

sub read_source {
    my ($path) = @_;
    open my $in, '<:raw', $path or die "open $path: $!\n";
    local $/;
    my $source = <$in>;
    close $in;
    return $source;
}

sub write_lines {
    my ($path, $lines) = @_;
    open my $out, '>:raw', $path or die "open $path: $!\n";
    print {$out} "$_\n" for @$lines;
    close $out;
}

sub has_atomics_wait_alias {
    my ($tokens) = @_;
    for my $index (0 .. $#$tokens) {
        return 1 if $tokens->[$index] eq '='
            && $index + 3 <= $#$tokens
            && $tokens->[$index + 1] eq 'Atomics'
            && $tokens->[$index + 2] eq '.'
            && $tokens->[$index + 3] eq 'wait'
            && ($index + 4 > $#$tokens || $tokens->[$index + 4] ne '(');
        return 1 if $tokens->[$index] =~ /\A(?:const|let|var)\z/
            && $index + 4 <= $#$tokens
            && $tokens->[$index + 1] =~ /\A[A-Za-z_\$][A-Za-z0-9_\$]*\z/
            && $tokens->[$index + 2] eq '='
            && $tokens->[$index + 3] eq 'Atomics'
            && $tokens->[$index + 4] =~ /\A(?:;|,)\z/;
        next unless $tokens->[$index] eq '}'
            && $index + 2 <= $#$tokens
            && $tokens->[$index + 1] eq '='
            && $tokens->[$index + 2] eq 'Atomics';
        my $depth = 0;
        for (my $start = $index; $start >= 0; $start--) {
            $depth++ if $tokens->[$start] eq '}';
            next unless $tokens->[$start] eq '{';
            $depth--;
            next unless $depth == 0;
            return 1 if grep { $_ eq 'wait' }
                @{$tokens}[$start + 1 .. $index - 1];
            last;
        }
    }
    return 0;
}

my %tagged;
open my $sab, '<:raw', $sab_ledger or die "open $sab_ledger: $!\n";
while (my $line = <$sab>) {
    next if $line =~ /^#/;
    $line =~ s/\n\z// or die "SAB ledger record lacks newline\n";
    my @field = split /\t/, $line, -1;
    next if $field[0] eq 'path';
    @field == 9 or die "SAB ledger schema drifted\n";
    $tagged{$field[0]} = 1 if $field[1] eq 'atomics-wait-no-agent';
}
close $sab;

my %atomics;
open my $atomics_in, '<:raw', $atomics_ledger or die "open $atomics_ledger: $!\n";
while (my $line = <$atomics_in>) {
    $line =~ s/\n\z// or die "Atomics ledger record lacks newline\n";
    my @field = split /\t/, $line, -1;
    next if $field[0] eq 'path';
    @field == 6 or die "Atomics ledger schema drifted\n";
    $atomics{$field[0]} = [@field[1 .. 5]];
}
close $atomics_in;

my (@all_js, @raw, @raw_sources, @selected, @noncall);
my (%source, %source_sha, %call_count, %raw_category);
my ($wait_async_raw, $wait_async_intersection, $bracket, $optional, $aliased) =
    (0, 0, 0, 0, 0);
my ($member_paths, $direct_paths, $direct_calls, $raw_only, $metadata_missing,
    $direct_excluded) = (0, 0, 0, 0, 0, 0);

find({
    no_chdir => 1,
    wanted => sub {
        return unless -f $File::Find::name && $File::Find::name =~ /\.js\z/;
        my $full = $File::Find::name;
        (my $path = $full) =~ s/^\Q$suite\E\///;
        push @all_js, $path;
        my $text = read_source($full);
        my $has_raw_wait = $text =~ /\bAtomics\s*\.\s*wait\b/ ? 1 : 0;
        my $has_raw_wait_async = $text =~ /\bAtomics\s*\.\s*waitAsync\b/ ? 1 : 0;
        $wait_async_raw++ if $has_raw_wait_async;
        $wait_async_intersection++ if $has_raw_wait && $has_raw_wait_async;
        $bracket++ if $text =~ /\bAtomics\s*\[\s*(['"])wait\1\s*\]/;
        $optional++ if $text =~ /\bAtomics\s*\?\.\s*wait\b/;
        my $tokens = index($text, 'Atomics') >= 0
            ? js_code_tokens($text, $path)
            : [];
        $aliased++ if has_atomics_wait_alias($tokens);
        return unless $has_raw_wait;

        push @raw, $path;
        my $digest = sha256_hex($text);
        push @raw_sources, "$path\t$digest";
        $source{$path} = $text;
        $source_sha{$path} = $digest;
        if (my $row = $atomics{$path}) {
            $raw_category{$row->[0]}++;
        } else {
            $metadata_missing++;
        }

        my ($members, $calls, $wait_async_calls, $agent_source) = (0, 0, 0, 0);
        for my $index (0 .. $#$tokens) {
            if ($index + 2 <= $#$tokens && $tokens->[$index] eq 'Atomics'
                    && $tokens->[$index + 1] eq '.'
                    && $tokens->[$index + 2] eq 'wait') {
                $members++;
                $calls++ if $index + 3 <= $#$tokens
                    && $tokens->[$index + 3] eq '(';
            }
            $wait_async_calls++ if $index + 3 <= $#$tokens
                && $tokens->[$index] eq 'Atomics'
                && $tokens->[$index + 1] eq '.'
                && $tokens->[$index + 2] eq 'waitAsync'
                && $tokens->[$index + 3] eq '(';
            $agent_source++ if $index + 2 <= $#$tokens
                && $tokens->[$index] eq '$262'
                && $tokens->[$index + 1] eq '.'
                && $tokens->[$index + 2] eq 'agent';
        }
        if ($members) {
            $member_paths++;
        } else {
            $raw_only++;
        }
        if ($members && !$calls) {
            push @noncall, $path;
        }
        return unless $calls;

        $direct_paths++;
        $direct_calls += $calls;
        $call_count{$path} = $calls;
        my ($frontmatter) = $text =~ m{/\*---(.*?)---\*/}s;
        $frontmatter //= '';
        my $agent_include =
            $frontmatter =~ /includes\s*:\s*\[[^\]]*\batomicsHelper\.js\b/s;
        my $wait_async_feature =
            $frontmatter =~ /features\s*:\s*\[[^\]]*\bAtomics\.waitAsync\b/s;
        if ($wait_async_calls || $agent_source || $agent_include || $wait_async_feature) {
            $direct_excluded++;
            return;
        }
        push @selected, $path;
    },
}, "$suite/test");

@all_js = sort @all_js;
@raw = sort @raw;
@raw_sources = sort @raw_sources;
@selected = sort @selected;
@noncall = sort @noncall;

my (@tagged_paths, @spillover_paths, @tagged_sources, @spillover_sources,
    @combined_sources, @tagged_keys, @spillover_keys, @combined_keys,
    @ledger_rows, @finite, @abrupt, @misfiled);
my (%tagged_features, %spillover_features, %combined_features);
my ($can_block_true, $can_block_false, $detach, $host_none, $finite_calls) =
    (0, 0, 0, 0, 0);

for my $path (@selected) {
    my $row = $atomics{$path} or die "selected path lacks Atomics metadata: $path\n";
    my ($category, $includes, $flags, $features, $recorded_sha) = @$row;
    die "selected source hash drifted: $path\n"
        unless $source_sha{$path} eq $recorded_sha;
    die "selected strictness drifted: $path\n"
        unless $flags eq '' || $flags eq 'CanBlockIsFalse'
            || $flags eq 'CanBlockIsTrue';

    my $is_tagged = $tagged{$path} ? 1 : 0;
    my $cohort = $is_tagged ? 'r3dh-tagged' : 'source-spillover';
    my $host = 'none';
    my $disposition = 'runnable';
    if ($flags eq 'CanBlockIsTrue') {
        $host = 'can-block:true';
        $can_block_true++;
        push @finite, $path;
        $finite_calls += $call_count{$path};
    } elsif ($flags eq 'CanBlockIsFalse') {
        $host = 'can-block:false';
        $disposition = 'unsupported-host-can-block-false';
        $can_block_false++;
        push @abrupt, $path;
    } elsif (index(",$includes,", ',detachArrayBuffer.js,') >= 0) {
        $host = 'detach-array-buffer';
        $detach++;
        push @abrupt, $path;
    } else {
        $host_none++;
        push @abrupt, $path;
    }
    push @misfiled, $path if $path =~ m{^test/built-ins/Atomics/notify/};

    my @features = grep { length } split /,/, $features, -1;
    $combined_features{$_} = 1 for @features;
    if ($is_tagged) {
        push @tagged_paths, $path;
        push @tagged_sources, "$path\t$recorded_sha";
        push @tagged_keys, "$path\tsloppy", "$path\tstrict";
        $tagged_features{$_} = 1 for @features;
    } else {
        push @spillover_paths, $path;
        push @spillover_sources, "$path\t$recorded_sha";
        push @spillover_keys, "$path\tsloppy", "$path\tstrict";
        $spillover_features{$_} = 1 for @features;
    }
    push @combined_sources, "$path\t$recorded_sha";
    push @combined_keys, "$path\tsloppy", "$path\tstrict";
    push @ledger_rows, join("\t", $path, $cohort, 'sloppy,strict',
        $includes, $flags, $features, $host, $disposition, $recorded_sha);
}

for my $list (\@tagged_paths, \@spillover_paths, \@tagged_sources,
        \@spillover_sources, \@combined_sources, \@tagged_keys,
        \@spillover_keys, \@combined_keys, \@ledger_rows, \@finite,
        \@abrupt, \@misfiled) {
    @$list = sort @$list;
}

write_lines("$outdir/all-js.txt", \@all_js);
write_lines("$outdir/raw.txt", \@raw);
write_lines("$outdir/raw-sources.tsv", \@raw_sources);
write_lines("$outdir/tagged.txt", \@tagged_paths);
write_lines("$outdir/spillover.txt", \@spillover_paths);
write_lines("$outdir/combined.txt", \@selected);
write_lines("$outdir/tagged-sources.tsv", \@tagged_sources);
write_lines("$outdir/spillover-sources.tsv", \@spillover_sources);
write_lines("$outdir/combined-sources.tsv", \@combined_sources);
write_lines("$outdir/tagged-keys.tsv", \@tagged_keys);
write_lines("$outdir/spillover-keys.tsv", \@spillover_keys);
write_lines("$outdir/combined-keys.tsv", \@combined_keys);
write_lines("$outdir/tagged.features", [sort keys %tagged_features]);
write_lines("$outdir/spillover.features", [sort keys %spillover_features]);
write_lines("$outdir/combined.features", [sort keys %combined_features]);
write_lines("$outdir/noncall.txt", \@noncall);
write_lines("$outdir/misfiled.txt", \@misfiled);
write_lines("$outdir/finite.txt", \@finite);
write_lines("$outdir/abrupt.txt", \@abrupt);

open my $ledger_out, '>:raw', "$outdir/ledger.tsv"
    or die "open generated ledger: $!\n";
print {$ledger_out} join("\t", qw(path cohort variants includes flags features
    host_requirements runner_disposition source_sha256)), "\n";
print {$ledger_out} "$_\n" for @ledger_rows;
close $ledger_out;

open my $summary, '>:raw', "$outdir/summary.txt"
    or die "open generated summary: $!\n";
my %summary = (
    suite_js_paths => scalar @all_js,
    raw_wait_member_paths => scalar @raw,
    raw_shared_agent_paths => $raw_category{'shared-agent'} // 0,
    raw_non_shared_paths => $raw_category{'non-shared-no-sab-tag'} // 0,
    raw_shared_can_block_false_paths => $raw_category{'shared-can-block-false'} // 0,
    raw_shared_no_extra_host_paths => $raw_category{'shared-no-extra-host'} // 0,
    code_wait_member_paths => $member_paths,
    code_direct_wait_paths => $direct_paths,
    code_direct_wait_calls => $direct_calls,
    code_non_call_member_paths => scalar @noncall,
    raw_only_paths => $raw_only,
    wait_async_raw_paths => $wait_async_raw,
    wait_async_intersection_paths => $wait_async_intersection,
    bracket_wait_paths => $bracket,
    optional_wait_paths => $optional,
    aliased_wait_paths => $aliased,
    metadata_missing_paths => $metadata_missing,
    tagged_paths => scalar @tagged_paths,
    tagged_variants => scalar @tagged_keys,
    tagged_features => scalar keys %tagged_features,
    spillover_paths => scalar @spillover_paths,
    spillover_variants => scalar @spillover_keys,
    spillover_features => scalar keys %spillover_features,
    combined_paths => scalar @selected,
    combined_variants => scalar @combined_keys,
    combined_features => scalar keys %combined_features,
    misfiled_paths => scalar @misfiled,
    finite_timeout_paths => scalar @finite,
    finite_timeout_calls => $finite_calls,
    pre_wait_abrupt_paths => scalar @abrupt,
    infinite_wait_paths => 0,
    not_equal_paths => 0,
    notify_wakeup_paths => 0,
    host_can_block_true_paths => $can_block_true,
    host_can_block_false_paths => $can_block_false,
    host_detach_array_buffer_paths => $detach,
    host_none_paths => $host_none,
    direct_excluded_paths => $direct_excluded,
);
print {$summary} "$_=$summary{$_}\n" for sort keys %summary;
close $summary;

open my $marker, '>:raw', "$outdir/perl-complete"
    or die "open completion marker: $!\n";
print {$marker} "complete\n";
close $marker;
__R3DJ_PERL_END__

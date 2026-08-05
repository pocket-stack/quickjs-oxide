#!/usr/bin/env bash
# Authenticate the R3di non-blocking shared Atomics tagged projection,
# source-audited spillover, selection-only Oxide run, and QuickJS receipts.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-shared-atomics-nonblocking-baseline.txt
sab_ledger=tests/test262-shared-array-buffer-universe.tsv
atomics_ledger=tests/test262-atomics-universe.tsv
tagged=tests/test262-shared-atomics-nonblocking-tagged.txt
spillover=tests/test262-shared-atomics-nonblocking-spillover.txt
spillover_ledger=tests/test262-shared-atomics-nonblocking-spillover.tsv
combined=tests/test262-shared-atomics-nonblocking.txt
profile=tests/test262-shared-atomics-nonblocking.conf
quickjs_receipt=tests/test262-shared-atomics-nonblocking-quickjs-receipt.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
workers=${TEST262_WORKERS:-8}
runner_override=${TEST262_RUNNER:-}
skip_unit_execution=${TEST262_SKIP_UNIT_EXECUTION:-false}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  authenticate both inventories and execute both engine receipts\n'
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
[[ "$skip_unit_execution" == false || "$skip_unit_execution" == true ]] \
    || { echo 'error: TEST262_SKIP_UNIT_EXECUTION must be true or false' >&2; exit 2; }
[[ -z "$runner_override" ]] \
    || { echo 'error: authenticated R3di gate does not accept TEST262_RUNNER' >&2; exit 2; }

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
generated_value() {
    awk -F= -v wanted="$1" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$generated_summary"
}
section() {
    awk -v wanted="[$2]" \
        '$0==wanted{inside=1;next} /^\[/{inside=0} inside&&NF&&$1!~/^#/{print}' \
        "$1"
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
check_file() {
    [[ -f "$1" && "$(lines "$1")" == "$2" && "$(sha "$1")" == "$3" ]] \
        || die "authenticated input drifted: $1"
}
header() {
    awk -F= -v wanted="# $2" \
        '$1==wanted{sub(/^[^=]*=/,"");print;found++} END{if(found!=1)exit 1}' \
        "$1"
}
json_metadata_string() {
    awk -v wanted="$2" '
        NR==1 {
            prefix="\"" wanted "\":\""
            start=index($0,prefix)
            if(!start)exit 1
            rest=substr($0,start+length(prefix))
            finish=index(rest,"\"")
            if(!finish)exit 1
            print substr(rest,1,finish-1)
            found++
        }
        END{if(found!=1)exit 1}
    ' "$1"
}
json_metadata_schema() {
    awk '
        NR==1&&match($0,/"schema":[0-9]+/){
            value=substr($0,RSTART,RLENGTH)
            sub(/^"schema":/,"",value)
            print value
            found++
        }
        END{if(found!=1)exit 1}
    ' "$1"
}
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_outcome_count() {
    report_rows "$1" | awk -F'\t' -v wanted="$2" '$7==wanted{count++} END{print count+0}'
}
report_other_outcome_count() {
    report_rows "$1" | awk -F'\t' -v wanted="$2" '$7!=wanted{count++} END{print count+0}'
}
json_outcome_count() {
    awk -v wanted="$2" '
        /^\{"kind":"result",/{
            if(!match($0,/"outcome":"[^"]*"/))exit 2
            value=substr($0,RSTART,RLENGTH)
            sub(/^"outcome":"/,"",value)
            sub(/"$/,"",value)
            if(value==wanted)count++
        }
        END{print count+0}
    ' "$1"
}
json_other_outcome_count() {
    awk -v wanted="$2" '
        /^\{"kind":"result",/{
            if(!match($0,/"outcome":"[^"]*"/))exit 2
            value=substr($0,RSTART,RLENGTH)
            sub(/^"outcome":"/,"",value)
            sub(/"$/,"",value)
            if(value!=wanted)count++
        }
        END{print count+0}
    ' "$1"
}
json_report_keys() {
    local report=$1
    awk -v report="$report" '
        function fail(message) {
            printf "error: R3di JSONL report %s: %s\n", report, message >"/dev/stderr"
            failed=1
            exit 2
        }
        /^\{"kind":"metadata",/ {
            metadata++
            if (NR != 1) fail("metadata record is not first")
            next
        }
        /^\{"kind":"result",/ {
            results++
            if (!match($0, /"path":"[^"]*"/)) fail("result is missing path")
            path=substr($0, RSTART, RLENGTH)
            sub(/^"path":"/, "", path)
            sub(/"$/, "", path)
            if (!match($0, /"variant":"[^"]*"/)) fail("result is missing variant")
            variant=substr($0, RSTART, RLENGTH)
            sub(/^"variant":"/, "", variant)
            sub(/"$/, "", variant)
            key=path "\t" variant
            if (seen[key]++) fail("duplicate result key")
            print key
            next
        }
        /^\{"kind":"summary",/ {
            summary++
            summary_line=NR
            next
        }
        { fail("unexpected record") }
        END {
            if (!failed && metadata != 1) fail("expected one metadata record")
            if (!failed && summary != 1) fail("expected one summary record")
            if (!failed && summary_line != NR) fail("summary record is not last")
        }
    ' "$report" | sort
}
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
check_static_inputs() {
    check_file "$baseline" 95 \
        a52b33e96d9bae60d8ad192f3e316c1917c259df6743f7baaca7ef66dcfe086b
    check_file "$sab_ledger" "$(value shared_array_buffer_universe_lines)" \
        "$(value shared_array_buffer_universe_sha256)"
    check_file "$atomics_ledger" "$(value atomics_universe_lines)" \
        "$(value atomics_universe_sha256)"
    check_file "$tagged" "$(value tagged_paths)" "$(value tagged_manifest_sha256)"
    check_file "$spillover" "$(value spillover_paths)" \
        "$(value spillover_manifest_sha256)"
    check_file "$spillover_ledger" "$(value spillover_ledger_lines)" \
        "$(value spillover_ledger_sha256)"
    check_file "$combined" "$(value combined_paths)" \
        "$(value combined_manifest_sha256)"
    check_file "$profile" "$(value scoped_profile_lines)" \
        "$(value scoped_profile_sha256)"
    check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
        "$(value quickjs_receipt_sha256)"
    [[ "$(value milestone_kind)" == scoped-implementation-receipt \
        && "$(value oxide_focused_report)" == authenticated \
        && "$(value implementation_parent_commit)" \
            == e578f8761c0d46c643f6e5b76167a48c256ef08e \
        && "$(git rev-parse --verify "$(value implementation_parent_commit)^{commit}" 2>/dev/null)" \
            == "$(value implementation_parent_commit)" \
        && "$(value oxide_focused_rows)" == "$(value combined_variants)" \
        && "$(value oxide_focused_report_lines)" == 211 \
        && "$(value oxide_focused_jsonl_lines)" == 202 \
        && "$(value oxide_focused_summary)" == pass=200 \
        && "$(value oxide_focused_passes)" == "$(value combined_variants)" \
        && "$(value oxide_focused_other_outcomes)" == 0 \
        && "$(value global_profile)" == "$live_profile" \
        && "$(value source_nonblocking_host_requirements)" == none \
        && "$(value tagged_metadata_only_path)" \
            == test/built-ins/Atomics/isLockFree/bigint/expected-return-value.js \
        && "$(value excluded_misfiled_wait_path)" \
            == test/built-ins/Atomics/notify/bigint/non-bigint64-typedarray-throws.js ]] \
        || die 'recorded R3di classification boundary drifted'
    [[ "$(value source_audited_rows)" == 123 \
        && "$(value source_excluded_wait_paths)" == 24 \
        && "$(value source_wait_async_paths)" == 0 \
        && "$(value source_agent_paths)" == 0 \
        && "$(value source_extra_host_paths)" == 0 \
        && "$(value source_direct_sab_paths)" == 89 \
        && "$(value source_helper_sab_paths)" == 10 \
        && "$(value quickjs_tagged_passes)" == "$(value tagged_variants)" \
        && "$(value quickjs_spillover_passes)" == "$(value spillover_variants)" \
        && "$(value quickjs_combined_passes)" == "$(value combined_variants)" \
        && "$(( $(value quickjs_tagged_passes) + $(value quickjs_spillover_passes) ))" \
            == "$(value quickjs_combined_passes)" \
        && "$(value quickjs_failed)" == 0 \
        && "$(value quickjs_skipped_feature)" == 0 ]] \
        || die 'recorded R3di source or QuickJS arithmetic boundary drifted'

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
            == "$(value test262_metadata_sha256)" \
        && "$(toml_test262_value "$upstream" oxide_profile)" == "$live_profile" \
        && "$(toml_test262_value "$upstream" oxide_profile_sha256)" \
            == "$(sha "$live_profile")" \
        && "$(lines "$live_profile")" == "$(value global_profile_lines)" \
        && "$(sha "$live_profile")" == "$(value global_profile_sha256)" \
        && "$(section "$live_profile" features | wc -l | tr -d '[:space:]')" \
            == "$(value global_profile_features)" \
        && "$(section "$live_profile" features | sha /dev/stdin)" \
            == "$(value global_profile_features_sha256)" ]] \
        || die 'compat/upstream.toml or global profile identity drifted'

    [[ "$(section "$profile" features | wc -l | tr -d '[:space:]')" \
            == "$(value scoped_profile_features)" \
        && "$(section "$profile" features | sha /dev/stdin)" \
            == "$(value scoped_profile_features_sha256)" \
        && -z "$(section "$profile" audited-negative-tests)" \
        && -z "$(section "$profile" execution)" \
        && "$(value selection_only)" == true \
        && "$(value oxide_focused_report)" == authenticated ]] \
        || die 'selection-only profile identity drifted'
}

cd -- "$root"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-shared-atomics-nonblocking.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
generated_summary=$tmp/generated-summary.txt
check_static_inputs

if [[ "$skip_unit_execution" == true ]]; then
    cargo test --locked --quiet --no-run --bin run-test262
else
    cargo test --locked --quiet --bin run-test262 shared_atomics_nonblocking
fi
cargo build --locked --release --quiet --bin run-test262
runner=${CARGO_TARGET_DIR:-$root/target}/release/run-test262
[[ -x "$runner" ]] || die "current-worktree Test262 runner is not executable: $runner"
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
git -C "$suite" diff --no-ext-diff --no-color --no-renames \
    --abbrev=7 --src-prefix=a/ --dst-prefix=b/ -- \
    harness/atomicsHelper.js harness/regExpUtils.js \
    | cmp -s - "$source_dir/tests/test262.patch" \
    || die 'prepared Test262 harness diff drifted'
[[ "$(grep -Fxc Atomics "$source_dir/test262.conf")" == 1 \
    && "$(grep -Fxc SharedArrayBuffer "$source_dir/test262.conf")" == 1 \
    && "$(grep -Fxc 'Atomics.waitAsync=skip' "$source_dir/test262.conf")" == 1 ]] \
    || die 'pinned QuickJS Atomics config boundary drifted'

metadata_bin=$tmp/metadata.bin
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(lines "$metadata_bin")" == "$(value test262_metadata_records)" \
    && "$(sha "$metadata_bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata inventory drifted'

command -v perl >/dev/null 2>&1 || die 'perl is required to authenticate source hashes'
generated_tagged=$tmp/tagged.txt
generated_spillover=$tmp/spillover.txt
generated_spillover_ledger=$tmp/spillover.tsv
generated_combined=$tmp/combined.txt
generated_tagged_sources=$tmp/tagged-sources.tsv
generated_tagged_keys=$tmp/tagged-keys.tsv
generated_source_nonblocking=$tmp/source-nonblocking.txt
generated_source_nonblocking_keys=$tmp/source-nonblocking-keys.tsv
generated_overlap=$tmp/overlap.txt
generated_spillover_sources=$tmp/spillover-sources.tsv
generated_spillover_keys=$tmp/spillover-keys.tsv
generated_combined_sources=$tmp/combined-sources.tsv
generated_combined_keys=$tmp/combined-keys.tsv
generated_features=$tmp/features.txt
perl -MDigest::SHA=sha256_hex - \
    "$suite" "$sab_ledger" "$atomics_ledger" \
    "$generated_tagged" "$generated_spillover" "$generated_spillover_ledger" \
    "$generated_combined" "$generated_tagged_sources" "$generated_tagged_keys" \
    "$generated_source_nonblocking" "$generated_source_nonblocking_keys" \
    "$generated_overlap" "$generated_spillover_sources" "$generated_spillover_keys" \
    "$generated_combined_sources" "$generated_combined_keys" \
    "$generated_features" "$generated_summary" <<'PERL'
use strict;
use warnings;
use Digest::SHA qw(sha256_hex);

my ($suite, $sab_ledger, $atomics_ledger, $tagged_out, $spillover_out,
    $spillover_ledger_out, $combined_out, $tagged_sources_out,
    $tagged_keys_out, $source_nonblocking_out, $source_nonblocking_keys_out,
    $overlap_out, $spillover_sources_out, $spillover_keys_out,
    $combined_sources_out, $combined_keys_out, $features_out, $summary_out) = @ARGV;

sub source_for {
    my ($path) = @_;
    open my $in, '<:raw', "$suite/$path" or die "open $suite/$path: $!\n";
    local $/;
    my $source = <$in>;
    close $in;
    return $source;
}

# Return only executable JavaScript tokens. Test262 frontmatter, comments,
# quoted strings, regexp bodies, and template literal text are deliberately
# absent; `${ ... }` expressions inside templates are tokenized recursively.
sub js_code_tokens {
    my ($source, $path) = @_;
    my @tokens;
    my $length = length $source;
    my $index = 0;
    my ($scan_code, $scan_template);

    my $skip_quoted = sub {
        my ($quote) = @_;
        $index++;
        while ($index < $length) {
            my $character = substr($source, $index, 1);
            if ($character eq '\\') {
                $index += 2;
                next;
            }
            $index++;
            return if $character eq $quote;
        }
        die "unterminated quoted literal while scanning $path\n";
    };

    my $skip_regexp = sub {
        $index++;
        my $in_class = 0;
        while ($index < $length) {
            my $character = substr($source, $index, 1);
            die "unterminated regexp literal while scanning $path\n"
                if $character eq "\n" || $character eq "\r";
            if ($character eq '\\') {
                $index += 2;
                next;
            }
            if ($character eq '[') {
                $in_class = 1;
            } elsif ($character eq ']' && $in_class) {
                $in_class = 0;
            } elsif ($character eq '/' && !$in_class) {
                $index++;
                $index++ while $index < $length
                    && substr($source, $index, 1) =~ /[A-Za-z]/;
                return;
            }
            $index++;
        }
        die "unterminated regexp literal while scanning $path\n";
    };

    $scan_template = sub {
        while ($index < $length) {
            my $character = substr($source, $index, 1);
            if ($character eq '\\') {
                $index += 2;
                next;
            }
            if ($character eq '`') {
                $index++;
                return;
            }
            if ($character eq '$' && substr($source, $index + 1, 1) eq '{') {
                $index += 2;
                $scan_code->(1);
                next;
            }
            $index++;
        }
        die "unterminated template literal while scanning $path\n";
    };

    $scan_code = sub {
        my ($template_expression) = @_;
        my $brace_depth = 0;
        my $regexp_allowed = 1;
        while ($index < $length) {
            my $character = substr($source, $index, 1);
            if ($character =~ /\s/) {
                $index++;
                next;
            }
            if ($character eq '/' && substr($source, $index + 1, 1) eq '/') {
                $index += 2;
                $index++ while $index < $length
                    && substr($source, $index, 1) ne "\n";
                next;
            }
            if ($character eq '/' && substr($source, $index + 1, 1) eq '*') {
                my $end = index($source, '*/', $index + 2);
                die "unterminated block comment while scanning $path\n" if $end < 0;
                $index = $end + 2;
                next;
            }
            if ($character eq "'" || $character eq '"') {
                $skip_quoted->($character);
                $regexp_allowed = 0;
                next;
            }
            if ($character eq '`') {
                $index++;
                $scan_template->();
                $regexp_allowed = 0;
                next;
            }
            if ($template_expression && $character eq '}' && $brace_depth == 0) {
                $index++;
                return;
            }
            if (substr($source, $index) =~ /\A([A-Za-z_\$][A-Za-z0-9_\$]*)/) {
                my $identifier = $1;
                push @tokens, $identifier;
                $index += length $identifier;
                $regexp_allowed = $identifier =~ /\A(?:await|case|delete|do|else|in|instanceof|new|return|throw|typeof|void|yield)\z/;
                next;
            }
            if (substr($source, $index) =~ /\A(?:\d|\.\d)/) {
                substr($source, $index) =~ /\A((?:0[xX][0-9A-Fa-f_]+|0[bB][01_]+|0[oO][0-7_]+|(?:\d[\d_]*\.?[\d_]*|\.\d[\d_]*)(?:[eE][+-]?[\d_]+)?)[n]?)/
                    or die "unrecognized numeric literal while scanning $path\n";
                $index += length $1;
                $regexp_allowed = 0;
                next;
            }
            if ($character eq '/') {
                if ($regexp_allowed) {
                    $skip_regexp->();
                    $regexp_allowed = 0;
                } else {
                    push @tokens, '/';
                    $index += substr($source, $index, 2) eq '/=' ? 2 : 1;
                    $regexp_allowed = 1;
                }
                next;
            }

            my $two = substr($source, $index, 2);
            my $punctuator = $two =~ /\A(?:\+\+|--|=>|==|!=|<=|>=|&&|\|\||\?\?|\?\.|\*\*|<<|>>|\+=|-=|\*=|%=|&=|\|=|\^=)\z/
                ? $two
                : $character;
            push @tokens, $punctuator;
            $index += length $punctuator;
            if ($punctuator eq '{') {
                $brace_depth++;
                $regexp_allowed = 1;
            } elsif ($punctuator eq '}') {
                $brace_depth-- if $brace_depth;
                $regexp_allowed = 0;
            } elsif ($punctuator =~ /\A(?:\)|\]|\+\+|--)\z/) {
                $regexp_allowed = 0;
            } else {
                $regexp_allowed = 1;
            }
        }
        die "unterminated template expression while scanning $path\n"
            if $template_expression;
    };

    $scan_code->(0);
    return \@tokens;
}

sub has_token_sequence {
    my ($tokens, @wanted) = @_;
    return 0 if @$tokens < @wanted;
    for my $start (0 .. $#$tokens - $#wanted) {
        my $matches = 1;
        for my $offset (0 .. $#wanted) {
            if ($tokens->[$start + $offset] ne $wanted[$offset]) {
                $matches = 0;
                last;
            }
        }
        return 1 if $matches;
    }
    return 0;
}

sub host_tokens {
    my ($tokens) = @_;
    my %hosts;
    for my $index (0 .. $#$tokens) {
        if ($tokens->[$index] eq '$262') {
            my $member = $index + 2 <= $#$tokens && $tokens->[$index + 1] eq '.'
                ? $tokens->[$index + 2]
                : '(direct)';
            $hosts{"\$262.$member"} = 1;
        }
        $hosts{$tokens->[$index]} = 1
            if $tokens->[$index] =~ /\A(?:CanBlock|canBlock|createRealm)\z/;
    }
    return sort keys %hosts;
}

sub operation_name {
    my ($path) = @_;
    $path =~ m{^test/built-ins/Atomics/([^/]+)/}
        or die "non-Atomics path in R3di closure: $path\n";
    return $1;
}

sub operation_counts {
    my ($paths) = @_;
    my %counts;
    $counts{operation_name($_)}++ for @$paths;
    return join ',', map { "$_:$counts{$_}" } sort keys %counts;
}

my (%tagged, %tagged_sha, %features, %tagged_ops);
my (@tagged_paths, @tagged_sources, @tagged_keys);
my $tagged_agent_notify = 0;
open my $sab, '<:raw', $sab_ledger or die "open $sab_ledger: $!\n";
while (my $line = <$sab>) {
    next if $line =~ /^#/;
    $line =~ s/\n\z// or die "SAB ledger row lacks newline\n";
    my @field = split /\t/, $line, -1;
    next if $field[0] eq 'path';
    @field == 9 or die "SAB ledger row has " . scalar(@field) . " fields\n";
    my ($path, $category, $variants, $includes, $flags, $feature_text,
        $hosts, $config, $source_sha) = @field;
    $tagged_agent_notify++
        if $category eq 'agent' && $path =~ m{^test/built-ins/Atomics/notify/};
    next unless $category eq 'atomics-nonblocking';
    die "tagged R3di row changed execution shape: $path\n"
        unless $variants eq 'sloppy,strict' && $flags eq ''
            && $hosts eq 'none' && $config eq 'runnable';
    my $source = source_for($path);
    die "tagged R3di source hash drifted: $path\n"
        unless sha256_hex($source) eq $source_sha;
    my $tokens = js_code_tokens($source, $path);
    die "tagged R3di row gained wait/agent work: $path\n"
        if has_token_sequence($tokens, ('Atomics', '.', 'wait', '('))
            || has_token_sequence($tokens, ('Atomics', '.', 'waitAsync', '('))
            || has_token_sequence($tokens, ('$262', '.', 'agent'));
    die "duplicate tagged R3di path: $path\n" if $tagged{$path}++;
    $tagged_sha{$path} = $source_sha;
    push @tagged_paths, $path;
    push @tagged_sources, "$path\t$source_sha";
    push @tagged_keys, "$path\tsloppy", "$path\tstrict";
    $tagged_ops{operation_name($path)}++;
    $features{$_} = 1 for grep { length } split /,/, $feature_text, -1;
}
close $sab;

my (%source_nonblocking, %source_sha, %spillover, %spillover_row, %excluded_wait);
my (@source_nonblocking_paths, @source_nonblocking_keys, @spillover_paths,
    @spillover_sources, @spillover_keys);
my $excluded_misfiled_wait =
    'test/built-ins/Atomics/notify/bigint/non-bigint64-typedarray-throws.js';
my ($source_audited_rows, $source_wait_async_paths, $source_agent_paths,
    $source_extra_host_paths, $source_direct_sab_paths,
    $source_helper_sab_paths) = (0, 0, 0, 0, 0, 0);
open my $atomics, '<:raw', $atomics_ledger or die "open $atomics_ledger: $!\n";
while (my $line = <$atomics>) {
    $line =~ s/\n\z// or die "Atomics ledger row lacks newline\n";
    my @field = split /\t/, $line, -1;
    next if $field[0] eq 'path';
    @field == 6 or die "Atomics ledger row has " . scalar(@field) . " fields\n";
    my ($path, $category, $includes, $flags, $feature_text, $recorded_sha) = @field;
    next unless $category eq 'shared-no-extra-host';
    $source_audited_rows++;
    my $source = source_for($path);
    die "source-audited Atomics hash drifted: $path\n"
        unless sha256_hex($source) eq $recorded_sha;
    my $tokens = js_code_tokens($source, $path);
    my $has_wait = has_token_sequence($tokens, ('Atomics', '.', 'wait', '('));
    my $has_wait_async = has_token_sequence($tokens, ('Atomics', '.', 'waitAsync', '('));
    my $has_agent = has_token_sequence($tokens, ('$262', '.', 'agent'))
        || $includes =~ /(?:^|,)atomicsHelper\.js(?:,|$)/;
    my @extra_hosts = host_tokens($tokens);
    push @extra_hosts, 'atomicsHelper.js'
        if $includes =~ /(?:^|,)atomicsHelper\.js(?:,|$)/;
    $source_wait_async_paths++ if $has_wait_async;
    $source_agent_paths++ if $has_agent;
    $source_extra_host_paths++ if @extra_hosts;
    die "shared-no-extra-host row gained waitAsync: $path\n" if $has_wait_async;
    die "shared-no-extra-host row gained an agent: $path\n" if $has_agent;
    die "shared-no-extra-host row gained host code (" . join(',', @extra_hosts) . "): $path\n"
        if @extra_hosts;
    if ($has_wait) {
        $excluded_wait{$path} = 1;
        next;
    }
    die "source-audited R3di row has flags: $path\n" if length $flags;
    my $direct_sab = has_token_sequence($tokens, ('new', 'SharedArrayBuffer', '('));
    my $helper_sab = $includes =~ /(?:^|,)testAtomics\.js(?:,|$)/
        && has_token_sequence($tokens, ('testWithAtomicsNonViewValues', '('));
    die "source-audited R3di row no longer evaluates SAB: $path\n"
        unless $direct_sab || $helper_sab;
    die "source-audited R3di row ambiguously uses direct and helper SAB: $path\n"
        if $direct_sab && $helper_sab;
    $source_direct_sab_paths++ if $direct_sab;
    $source_helper_sab_paths++ if $helper_sab;
    die "duplicate source-audited R3di path: $path\n"
        if $source_nonblocking{$path}++;
    $source_sha{$path} = $recorded_sha;
    push @source_nonblocking_paths, $path;
    push @source_nonblocking_keys, "$path\tsloppy", "$path\tstrict";
    next if $tagged{$path};
    $spillover{$path} = 1;
    $spillover_row{$path} = join("\t", $path, $category, 'sloppy,strict',
        $includes, $flags, $feature_text, $recorded_sha);
    push @spillover_paths, $path;
    push @spillover_sources, "$path\t$recorded_sha";
    push @spillover_keys, "$path\tsloppy", "$path\tstrict";
    $features{$_} = 1 for grep { length } split /,/, $feature_text, -1;
}
close $atomics;

die "synchronous wait exclusion count drifted\n"
    unless scalar(keys %excluded_wait) == 24;
die "misfiled synchronous wait is no longer in the code-derived excluded set\n"
    unless $excluded_wait{$excluded_misfiled_wait};
my @overlap = sort grep { $source_nonblocking{$_} } keys %tagged;
my @tagged_only = sort grep { !$source_nonblocking{$_} } keys %tagged;
die "tagged metadata-only boundary drifted\n"
    unless @tagged_only == 1
        && $tagged_only[0] eq
            'test/built-ins/Atomics/isLockFree/bigint/expected-return-value.js';

@tagged_paths = sort @tagged_paths;
@tagged_sources = sort @tagged_sources;
@tagged_keys = sort @tagged_keys;
@source_nonblocking_paths = sort @source_nonblocking_paths;
@source_nonblocking_keys = sort @source_nonblocking_keys;
@spillover_paths = sort @spillover_paths;
@spillover_sources = sort @spillover_sources;
@spillover_keys = sort @spillover_keys;
my @combined_paths = sort { $a cmp $b } (@tagged_paths, @spillover_paths);
my @combined_sources = sort { $a cmp $b } (
    map({ "$_\t$tagged_sha{$_}" } @tagged_paths),
    map({ "$_\t$source_sha{$_}" } @spillover_paths),
);
my @combined_keys = sort map { ("$_\tsloppy", "$_\tstrict") } @combined_paths;

my %seen_combined;
die "combined R3di closure contains duplicates\n"
    if grep { $seen_combined{$_}++ } @combined_paths;

sub write_lines {
    my ($path, $lines) = @_;
    open my $out, '>:raw', $path or die "open $path: $!\n";
    print {$out} "$_\n" for @$lines;
    close $out;
}

write_lines($tagged_out, \@tagged_paths);
write_lines($spillover_out, \@spillover_paths);
write_lines($combined_out, \@combined_paths);
write_lines($tagged_sources_out, \@tagged_sources);
write_lines($tagged_keys_out, \@tagged_keys);
write_lines($source_nonblocking_out, \@source_nonblocking_paths);
write_lines($source_nonblocking_keys_out, \@source_nonblocking_keys);
write_lines($overlap_out, \@overlap);
write_lines($spillover_sources_out, \@spillover_sources);
write_lines($spillover_keys_out, \@spillover_keys);
write_lines($combined_sources_out, \@combined_sources);
write_lines($combined_keys_out, \@combined_keys);
my @feature_list = sort keys %features;
write_lines($features_out, \@feature_list);

open my $spill_ledger, '>:raw', $spillover_ledger_out
    or die "open $spillover_ledger_out: $!\n";
print {$spill_ledger} "# source_ledger=tests/test262-atomics-universe.tsv\n";
print {$spill_ledger} "# source_rule=shared-no-extra-host and code-only SAB evaluation without wait/waitAsync/host, minus tagged projection\n";
print {$spill_ledger} "# excluded_wait_path=$excluded_misfiled_wait\n";
print {$spill_ledger} join("\t", qw(path category variants includes flags features source_sha256)), "\n";
print {$spill_ledger} "$spillover_row{$_}\n" for @spillover_paths;
close $spill_ledger;

open my $summary, '>:raw', $summary_out or die "open $summary_out: $!\n";
print {$summary} "tagged.paths=" . scalar(@tagged_paths) . "\n";
print {$summary} "tagged.variants=" . scalar(@tagged_keys) . "\n";
print {$summary} "tagged.operations=" . operation_counts(\@tagged_paths) . "\n";
print {$summary} "tagged.agent_notify=$tagged_agent_notify\n";
print {$summary} "source.audited_rows=$source_audited_rows\n";
print {$summary} "source.excluded_wait_paths=" . scalar(keys %excluded_wait) . "\n";
print {$summary} "source.wait_async_paths=$source_wait_async_paths\n";
print {$summary} "source.agent_paths=$source_agent_paths\n";
print {$summary} "source.extra_host_paths=$source_extra_host_paths\n";
print {$summary} "source.direct_sab_paths=$source_direct_sab_paths\n";
print {$summary} "source.helper_sab_paths=$source_helper_sab_paths\n";
print {$summary} "source.paths=" . scalar(@source_nonblocking_paths) . "\n";
print {$summary} "source.variants=" . scalar(@source_nonblocking_keys) . "\n";
print {$summary} "overlap.paths=" . scalar(@overlap) . "\n";
print {$summary} "tagged_only.paths=" . scalar(@tagged_only) . "\n";
print {$summary} "spillover.paths=" . scalar(@spillover_paths) . "\n";
print {$summary} "spillover.variants=" . scalar(@spillover_keys) . "\n";
print {$summary} "spillover.operations=" . operation_counts(\@spillover_paths) . "\n";
print {$summary} "combined.paths=" . scalar(@combined_paths) . "\n";
print {$summary} "combined.variants=" . scalar(@combined_keys) . "\n";
print {$summary} "combined.operations=" . operation_counts(\@combined_paths) . "\n";
print {$summary} "features=" . scalar(@feature_list) . "\n";
close $summary;
PERL

diff -u "$tagged" "$generated_tagged" \
    || die 'tagged non-blocking Atomics projection drifted'
diff -u "$spillover" "$generated_spillover" \
    || die 'source-audited spillover manifest drifted'
diff -u "$spillover_ledger" "$generated_spillover_ledger" \
    || die 'source-audited spillover ledger drifted'
diff -u "$combined" "$generated_combined" \
    || die 'combined non-blocking Atomics closure drifted'
diff -u <(section "$profile" features) "$generated_features" \
    || die 'scoped profile is not the exact combined feature union'

[[ "$(generated_value tagged.paths)" == "$(value tagged_paths)" \
    && "$(generated_value tagged.variants)" == "$(value tagged_variants)" \
    && "$(generated_value tagged.operations)" == "$(value tagged_operation_counts)" \
    && "$(generated_value tagged.agent_notify)" == "$(value tagged_agent_notify_paths)" \
    && "$(generated_value source.audited_rows)" == "$(value source_audited_rows)" \
    && "$(generated_value source.excluded_wait_paths)" \
        == "$(value source_excluded_wait_paths)" \
    && "$(generated_value source.wait_async_paths)" == "$(value source_wait_async_paths)" \
    && "$(generated_value source.agent_paths)" == "$(value source_agent_paths)" \
    && "$(generated_value source.extra_host_paths)" \
        == "$(value source_extra_host_paths)" \
    && "$(generated_value source.direct_sab_paths)" \
        == "$(value source_direct_sab_paths)" \
    && "$(generated_value source.helper_sab_paths)" \
        == "$(value source_helper_sab_paths)" \
    && "$(generated_value source.paths)" == "$(value source_nonblocking_paths)" \
    && "$(generated_value source.variants)" == "$(value source_nonblocking_variants)" \
    && "$(generated_value overlap.paths)" == "$(value tagged_paths_in_source_closure)" \
    && "$(generated_value tagged_only.paths)" == 1 \
    && "$(generated_value spillover.paths)" == "$(value spillover_paths)" \
    && "$(generated_value spillover.variants)" == "$(value spillover_variants)" \
    && "$(generated_value spillover.operations)" == "$(value spillover_operation_counts)" \
    && "$(generated_value combined.paths)" == "$(value combined_paths)" \
    && "$(generated_value combined.variants)" == "$(value combined_variants)" \
    && "$(generated_value combined.operations)" == "$(value combined_operation_counts)" \
    && "$(generated_value features)" == "$(value scoped_profile_features)" \
    && "$(sha "$generated_tagged_sources")" == "$(value tagged_source_projection_sha256)" \
    && "$(sha "$generated_tagged_keys")" == "$(value tagged_keys_sha256)" \
    && "$(sha "$generated_source_nonblocking")" \
        == "$(value source_nonblocking_paths_sha256)" \
    && "$(sha "$generated_source_nonblocking_keys")" \
        == "$(value source_nonblocking_keys_sha256)" \
    && "$(sha "$generated_overlap")" == "$(value tagged_source_overlap_sha256)" \
    && "$(sha "$generated_spillover_sources")" \
        == "$(value spillover_source_projection_sha256)" \
    && "$(sha "$generated_spillover_keys")" == "$(value spillover_keys_sha256)" \
    && "$(sha "$generated_combined_sources")" \
        == "$(value combined_source_projection_sha256)" \
    && "$(sha "$generated_combined_keys")" == "$(value combined_keys_sha256)" \
    && "$(sha "$generated_features")" == "$(value scoped_profile_features_sha256)" ]] \
    || die 'R3di path, source, key, feature, or operation projection drifted'

rejection_log=$tmp/rejected.log
expect_rejected_selection() {
    local expected=$1
    shift
    if "$runner" --suite "$suite" --config "$source_dir/test262.conf" \
        --oxide-profile "$profile" "$@" --report "$tmp/rejected.tsv" \
        --mode both >"$rejection_log" 2>&1; then
        die "selection-only profile unexpectedly accepted: $*"
    fi
    grep -Fq "$expected" "$rejection_log" \
        || { tail -n 100 "$rejection_log" >&2; die "unexpected selection rejection: $*"; }
}
expect_rejected_selection 'requires its pinned manifest' --all
expect_rejected_selection 'requires its pinned manifest' \
    --test test/built-ins/Atomics/add/good-views.js
expect_rejected_selection 'requires tests/test262-shared-atomics-nonblocking.txt' \
    --manifest "$tagged"
expect_rejected_selection 'requires tests/test262-shared-atomics-nonblocking.txt' \
    --manifest "$spillover"
expect_rejected_selection 'requires tests/test262-shared-atomics-nonblocking.txt' \
    --manifest "$sab_ledger"

focused_report=$tmp/focused.tsv
focused_json=${focused_report%.tsv}.jsonl
focused_json_keys=$tmp/focused-json-keys.tsv
focused_log=$tmp/focused.log
"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$combined" --report "$focused_report" \
    --mode both --timeout-ms 30000 --workers "$workers" --allow-failures \
    >"$focused_log" 2>&1 \
    || { tail -n 100 "$focused_log" >&2; die 'exact combined selection was not accepted'; }
json_report_keys "$focused_json" >"$focused_json_keys" \
    || die 'authenticated Oxide JSONL report is malformed'
expected_json_summary=$(printf '{"kind":"summary","outcomes":{"pass":%s}}' \
    "$(value oxide_focused_passes)")
[[ -f "$focused_report" && -f "$focused_json" \
    && "$(head -n 1 "$focused_report")" == '# quickjs-oxide Test262 outcome vector v2' \
    && "$(json_metadata_schema "$focused_json")" == 2 \
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
    && "$(json_metadata_string "$focused_json" quickjs)" \
        == "$(header "$focused_report" quickjs)" \
    && "$(json_metadata_string "$focused_json" test262)" \
        == "$(header "$focused_report" test262)" \
    && "$(json_metadata_string "$focused_json" test262_patch_sha256)" \
        == "$(header "$focused_report" test262_patch_sha256)" \
    && "$(json_metadata_string "$focused_json" test262_config_sha256)" \
        == "$(header "$focused_report" test262_config_sha256)" \
    && "$(json_metadata_string "$focused_json" test262_metadata_sha256)" \
        == "$(header "$focused_report" test262_metadata_sha256)" \
    && "$(json_metadata_string "$focused_json" oxide_profile_sha256)" \
        == "$(header "$focused_report" oxide_profile_sha256)" \
    && "$(json_metadata_string "$focused_json" profile)" \
        == "$(header "$focused_report" profile)" \
    && "$(json_metadata_string "$focused_json" mode)" \
        == "$(header "$focused_report" mode)" \
    && "$(report_rows "$focused_report" | wc -l | tr -d '[:space:]')" \
        == "$(value oxide_focused_rows)" \
    && "$(report_keys "$focused_report" | sha /dev/stdin)" \
        == "$(value combined_keys_sha256)" \
    && "$(lines "$focused_json_keys")" == "$(value combined_variants)" \
    && "$(sha "$focused_json_keys")" == "$(value combined_keys_sha256)" \
    && "$(lines "$focused_report")" == "$(value oxide_focused_report_lines)" \
    && "$(lines "$focused_json")" == "$(value oxide_focused_jsonl_lines)" \
    && "$(sha "$focused_report")" == "$(value oxide_focused_tsv_sha256)" \
    && "$(sha "$focused_json")" == "$(value oxide_focused_jsonl_sha256)" \
    && "$(report_summary "$focused_report")" == "$(computed_summary "$focused_report")" \
    && "$(report_summary "$focused_report")" == "$(value oxide_focused_summary)" \
    && "$(report_outcome_count "$focused_report" pass)" \
        == "$(value oxide_focused_passes)" \
    && "$(report_other_outcome_count "$focused_report" pass)" \
        == "$(value oxide_focused_other_outcomes)" \
    && "$(json_outcome_count "$focused_json" pass)" \
        == "$(value oxide_focused_passes)" \
    && "$(json_other_outcome_count "$focused_json" pass)" \
        == "$(value oxide_focused_other_outcomes)" \
    && "$(tail -n 1 "$focused_json")" == "$expected_json_summary" ]] \
    || die 'authenticated Oxide focused report identity drifted'
cmp -s <(report_keys "$focused_report") "$focused_json_keys" \
    || die 'authenticated Oxide TSV and JSONL result keys diverged'
report_rows "$focused_report" | awk -F'\t' '$8=="selection"{exit 1}' \
    || die 'exact R3di profile left a row at the selection phase'

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
run_quickjs_cohort() {
    local manifest=$1
    local expected_variants=$2
    local log=$3
    local label=$4
    local result_variable=$5
    local files=()
    local actual_variants
    while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$manifest"
    if ! (cd "$source_dir" && ./run-test262 -m -c test262.conf -a \
            -T "$workers" -f "${files[@]}") >"$log" 2>&1; then
        tail -n 100 "$log" >&2
        die "pinned QuickJS failed the $label cohort"
    fi
    if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$log" \
        || [[ "$(grep -Ec '^Average memory statistics for [0-9]+ tests:$' "$log")" != 1 ]]; then
        tail -n 100 "$log" >&2
        die "pinned QuickJS $label receipt drifted"
    fi
    actual_variants=$(sed -n \
        's/^Average memory statistics for \([0-9][0-9]*\) tests:$/\1/p' "$log")
    [[ "$actual_variants" == "$expected_variants" ]] \
        || die "pinned QuickJS $label executed $actual_variants variants, expected $expected_variants"
    printf -v "$result_variable" '%s' "$actual_variants"
}
run_quickjs_cohort "$tagged" "$(value tagged_variants)" \
    "$tmp/quickjs-tagged.log" 'tagged non-blocking Atomics' quickjs_tagged_actual
run_quickjs_cohort "$spillover" "$(value spillover_variants)" \
    "$tmp/quickjs-spillover.log" 'source-audited spillover' quickjs_spillover_actual
quickjs_combined_actual=$((quickjs_tagged_actual + quickjs_spillover_actual))
[[ "$quickjs_tagged_actual" == "$(value tagged_variants)" \
    && "$quickjs_spillover_actual" == "$(value spillover_variants)" \
    && "$quickjs_combined_actual" == "$(value combined_variants)" \
    && "$quickjs_tagged_actual" == "$(value quickjs_tagged_passes)" \
    && "$quickjs_spillover_actual" == "$(value quickjs_spillover_passes)" \
    && "$quickjs_combined_actual" == "$(value quickjs_combined_passes)" ]] \
    || die 'pinned QuickJS combined pass arithmetic drifted'

{
    echo '# Pinned QuickJS oracle receipt for the R3di non-blocking shared Atomics closure.'
    echo "quickjs=$(value quickjs)"
    echo "test262=$(value test262)"
    echo "shared_array_buffer_universe_sha256=$(value shared_array_buffer_universe_sha256)"
    echo "atomics_universe_sha256=$(value atomics_universe_sha256)"
    echo "tagged_manifest_sha256=$(value tagged_manifest_sha256)"
    echo "spillover_manifest_sha256=$(value spillover_manifest_sha256)"
    echo "combined_manifest_sha256=$(value combined_manifest_sha256)"
    echo "tagged_paths=$(value tagged_paths)"
    echo "tagged_variants=$(value tagged_variants)"
    echo "tagged_passes=$quickjs_tagged_actual"
    echo "spillover_paths=$(value spillover_paths)"
    echo "spillover_variants=$(value spillover_variants)"
    echo "spillover_passes=$quickjs_spillover_actual"
    echo "combined_paths=$(value combined_paths)"
    echo "combined_variants=$(value combined_variants)"
    echo "combined_passes=$quickjs_combined_actual"
    echo 'failed=0'
    echo 'skipped_feature=0'
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
diff -u "$quickjs_receipt" "$tmp/quickjs-receipt.txt" \
    || die 'pinned QuickJS receipt projection drifted'

echo "R3di implementation gate verified: tagged 78 / 156; source spillover 22 / 44; combined 100 / 200; authenticated Oxide $(report_summary "$focused_report"); pinned QuickJS 200 / 200."

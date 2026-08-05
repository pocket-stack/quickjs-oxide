#!/usr/bin/env bash
# Authenticate the R3dh SharedArrayBuffer metadata universe, selection-only
# core frontier, Oxide pass vector, and pinned QuickJS oracle receipt.

set -euo pipefail
export LC_ALL=C
export TZ=America/Los_Angeles

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$script_dir/.." && pwd)
baseline=tests/test262-shared-array-buffer-baseline.txt
ledger=tests/test262-shared-array-buffer-universe.tsv
core=tests/test262-shared-array-buffer-core.txt
profile=tests/test262-shared-array-buffer-core.conf
quickjs_receipt=tests/test262-shared-array-buffer-quickjs-receipt.txt
upstream=compat/upstream.toml
live_profile=compat/test262-oxide.conf
oracle_log=target/test262-shared-array-buffer-quickjs.log
workers=${TEST262_WORKERS:-8}
runner_override=${TEST262_RUNNER:-}
skip_unit_execution=${TEST262_SKIP_UNIT_EXECUTION:-false}

usage() {
    printf 'usage: %s [--check]\n' "${0##*/}"
    printf '  --check  authenticate the inventory and both engine receipts\n'
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
report_rows() { awk -F'\t' '!/^#/&&!($1=="path"&&$2=="variant")' "$1"; }
report_keys() { report_rows "$1" | awk -F'\t' '{print $1 "\t" $2}' | sort; }
report_summary() { tail -n 1 "$1" | sed 's/^# summary //'; }
computed_summary() {
    report_rows "$1" | awk -F'\t' '{print $7}' | sort | uniq -c | awk '
        {out=out (NR==1?"":" ") $2 "=" $1} END{print out}'
}
absolute_from_root() {
    case $1 in
        /*) printf '%s\n' "$1" ;;
        *) printf '%s/%s\n' "$root" "$1" ;;
    esac
}

check_static_inputs() {
    check_file "$baseline" 82 \
        c386f99d7b14da071e8f684eb5e8883084ac7f9a64bb11c1921ef2ec001d5afe
    check_file "$ledger" "$(value universe_ledger_lines)" \
        "$(value universe_ledger_sha256)"
    check_file "$core" "$(value core_manifest_lines)" \
        "$(value core_manifest_sha256)"
    check_file "$profile" "$(value scoped_profile_lines)" \
        "$(value scoped_profile_sha256)"
    check_file "$quickjs_receipt" "$(value quickjs_receipt_lines)" \
        "$(value quickjs_receipt_sha256)"
    sort -c "$core" || die 'core manifest is not bytewise sorted'
    [[ -z "$(sort "$core" | uniq -d)" ]] || die 'core manifest contains duplicates'

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
            == "$(sha "$live_profile")" ]] \
        || die 'compat/upstream.toml identity drifted'

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
tmp=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-shared-array-buffer.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT HUP INT TERM
generated_summary=$tmp/generated-summary.txt
check_static_inputs

if [[ "$skip_unit_execution" == true ]]; then
    cargo test --locked --quiet --no-run --bin run-test262
else
    cargo test --locked --quiet --bin run-test262 shared_array_buffer
fi
if [[ -n "$runner_override" ]]; then
    runner=$(absolute_from_root "$runner_override")
    [[ -x "$runner" ]] || die "TEST262_RUNNER is not executable: $runner"
else
    cargo build --locked --release --quiet --bin run-test262
    runner=$root/target/release/run-test262
fi
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
    || die 'pinned QuickJS SharedArrayBuffer config boundary drifted'

metadata_bin=$tmp/metadata.bin
"$runner" --suite "$suite" --validate-metadata "$metadata_bin" >/dev/null
[[ "$(lines "$metadata_bin")" == "$(value test262_metadata_records)" \
    && "$(sha "$metadata_bin")" == "$(value test262_metadata_sha256)" ]] \
    || die 'pinned Test262 metadata inventory drifted'

command -v perl >/dev/null 2>&1 || die 'perl is required to authenticate source hashes'
generated_ledger=$tmp/universe.tsv
generated_core=$tmp/core.txt
perl -MDigest::SHA=sha256_hex - \
    "$suite" "$metadata_bin" "$generated_ledger" "$generated_core" \
    "$generated_summary" "$(value quickjs)" "$(value test262)" \
    "$(value test262_patch_sha256)" "$(value test262_config_sha256)" \
    "$(value test262_metadata_sha256)" <<'PERL'
use strict;
use warnings;

my ($suite, $metadata, $ledger, $core, $summary, $quickjs, $test262,
    $patch_sha, $config_sha, $metadata_sha) = @ARGV;
open my $in, '<:raw', $metadata or die "open $metadata: $!\n";
open my $out, '>:raw', $ledger or die "open $ledger: $!\n";
open my $core_out, '>:raw', $core or die "open $core: $!\n";
print {$out} "# quickjs=$quickjs\n";
print {$out} "# test262=$test262\n";
print {$out} "# test262_patch_sha256=$patch_sha\n";
print {$out} "# test262_config_sha256=$config_sha\n";
print {$out} "# test262_metadata_sha256=$metadata_sha\n";
print {$out} join("\t", qw(path category variants includes flags features
    host_requirements config_disposition source_sha256)), "\n";

my (%paths, %variants, %flags, %hosts, %disposition, %disposition_variants);
my ($total_paths, $total_variants, $unflagged) = (0, 0, 0);
my $previous = '';
while (my $record = <$in>) {
    $record =~ s/\n\z// or die "metadata record lacks newline\n";
    my @field = split /\0/, $record, -1;
    @field == 6 or die "metadata record has " . scalar(@field) . " fields\n";
    my ($path, $includes, $flag_text, $feature_text, $phase, $type) = @field;
    my %feature = map { $_ => 1 } grep { length } split /,/, $feature_text, -1;
    next unless $feature{'SharedArrayBuffer'};

    die "SAB metadata order drifted at $path\n"
        if $previous ne '' && $previous ge $path;
    $previous = $path;
    die "SAB universe gained a negative: $path\n" if length($phase) || length($type);
    die "unsafe ledger field: $path\n" if grep { /[\t\r\n]/ } @field;

    my @flag = grep { length } split /,/, $flag_text, -1;
    my %flag = map { $_ => 1 } @flag;
    my %known_flag = map { $_ => 1 }
        qw(CanBlockIsFalse CanBlockIsTrue async generated noStrict onlyStrict);
    die "SAB universe gained unknown flag at $path\n"
        if grep { !$known_flag{$_} } @flag;
    die "conflicting strictness flags at $path\n"
        if $flag{noStrict} && $flag{onlyStrict};
    my $variant_text = $flag{noStrict} ? 'sloppy'
        : $flag{onlyStrict} ? 'strict' : 'sloppy,strict';
    my $variant_count = $variant_text =~ /,/ ? 2 : 1;

    my $source_path = "$suite/$path";
    open my $source_in, '<:raw', $source_path or die "open $source_path: $!\n";
    local $/;
    my $source = <$source_in>;
    close $source_in;
    my %include = map { $_ => 1 } grep { length } split /,/, $includes, -1;
    my $agent_source = $source =~ /\$262\.agent\b/ ? 1 : 0;
    my $agent_include = $include{'atomicsHelper.js'} ? 1 : 0;
    die "agent source/include mismatch at $path\n"
        if $agent_source != $agent_include;
    my $create_realm = $source =~ /\$262\.createRealm\b/ ? 1 : 0;
    my $wait_async = $feature{'Atomics.waitAsync'} ? 1 : 0;
    my $wait = $source =~ /\bAtomics\s*\.\s*wait\s*\(/ ? 1 : 0;
    my $atomics = $feature{Atomics} || $source =~ /\bAtomics\b/;
    my $category = $wait_async ? 'wait-async'
        : $agent_source ? 'agent'
        : $wait ? 'atomics-wait-no-agent'
        : $atomics ? 'atomics-nonblocking' : 'sab-core';
    die "sab-core contains Atomics at $path\n"
        if $category eq 'sab-core' && ($feature{Atomics} || $source =~ /\bAtomics\b/);
    die "CanBlock flag outside synchronous wait at $path\n"
        if ($flag{CanBlockIsFalse} || $flag{CanBlockIsTrue})
            && $category ne 'atomics-wait-no-agent';
    die "async flag outside waitAsync at $path\n"
        if $flag{async} && $category ne 'wait-async';

    my @host;
    push @host, 'agent' if $agent_source;
    push @host, 'create-realm' if $create_realm;
    push @host, 'can-block:false' if $flag{CanBlockIsFalse};
    push @host, 'can-block:true' if $flag{CanBlockIsTrue};
    my $host_text = @host ? join(',', @host) : 'none';
    my $config = $wait_async ? 'skip-feature:Atomics.waitAsync' : 'runnable';
    my $source_sha = sha256_hex($source);
    print {$out} join("\t", $path, $category, $variant_text, $includes,
        $flag_text, $feature_text, $host_text, $config, $source_sha), "\n";
    print {$core_out} "$path\n" if $category eq 'sab-core';

    $paths{$category}++;
    $variants{$category} += $variant_count;
    $disposition{$config}++;
    $disposition_variants{$config} += $variant_count;
    $total_paths++;
    $total_variants += $variant_count;
    $unflagged++ unless @flag;
    $flags{$_}++ for @flag;
    $hosts{$_}++ for @host;
}
close $in;
close $out;
close $core_out;

open my $sum, '>:raw', $summary or die "open $summary: $!\n";
print {$sum} "paths=$total_paths\nvariants=$total_variants\n";
for my $category (qw(sab-core atomics-nonblocking atomics-wait-no-agent agent wait-async)) {
    print {$sum} "$category.paths=" . ($paths{$category} // 0) . "\n";
    print {$sum} "$category.variants=" . ($variants{$category} // 0) . "\n";
}
print {$sum} "flag.unflagged=$unflagged\n";
for my $key (sort keys %flags) {
    print {$sum} "flag.$key=$flags{$key}\n";
}
for my $key (sort keys %hosts) {
    print {$sum} "host.$key=$hosts{$key}\n";
}
for my $key (sort keys %disposition) {
    print {$sum} "config.$key.paths=$disposition{$key}\n";
    print {$sum} "config.$key.variants=$disposition_variants{$key}\n";
}
close $sum;
PERL

diff -u "$ledger" "$generated_ledger" \
    || die 'SharedArrayBuffer universe ledger drifted'
diff -u "$core" "$generated_core" \
    || die 'SharedArrayBuffer core manifest drifted'

[[ "$(generated_value paths)" == "$(value universe_paths)" \
    && "$(generated_value variants)" == "$(value universe_variants)" \
    && "$(generated_value sab-core.paths)" == "$(value sab_core_paths)" \
    && "$(generated_value sab-core.variants)" == "$(value sab_core_variants)" \
    && "$(generated_value atomics-nonblocking.paths)" == "$(value atomics_nonblocking_paths)" \
    && "$(generated_value atomics-nonblocking.variants)" == "$(value atomics_nonblocking_variants)" \
    && "$(generated_value atomics-wait-no-agent.paths)" == "$(value atomics_wait_no_agent_paths)" \
    && "$(generated_value atomics-wait-no-agent.variants)" == "$(value atomics_wait_no_agent_variants)" \
    && "$(generated_value agent.paths)" == "$(value agent_paths)" \
    && "$(generated_value agent.variants)" == "$(value agent_variants)" \
    && "$(generated_value wait-async.paths)" == "$(value wait_async_paths)" \
    && "$(generated_value wait-async.variants)" == "$(value wait_async_variants)" \
    && "$(generated_value flag.CanBlockIsFalse)" == "$(value flag_CanBlockIsFalse_paths)" \
    && "$(generated_value flag.CanBlockIsTrue)" == "$(value flag_CanBlockIsTrue_paths)" \
    && "$(generated_value flag.async)" == "$(value flag_async_paths)" \
    && "$(generated_value flag.generated)" == "$(value flag_generated_paths)" \
    && "$(generated_value flag.noStrict)" == "$(value flag_noStrict_paths)" \
    && "$(generated_value flag.onlyStrict)" == "$(value flag_onlyStrict_paths)" \
    && "$(generated_value flag.unflagged)" == "$(value unflagged_paths)" \
    && "$(generated_value host.agent)" == "$(value host_agent_paths)" \
    && "$(generated_value host.create-realm)" == "$(value host_create_realm_paths)" \
    && "$(generated_value host.can-block:false)" == "$(value host_can_block_false_paths)" \
    && "$(generated_value host.can-block:true)" == "$(value host_can_block_true_paths)" \
    && "$(generated_value config.runnable.paths)" == "$(value config_runnable_paths)" \
    && "$(generated_value config.runnable.variants)" == "$(value config_runnable_variants)" \
    && "$(generated_value config.skip-feature:Atomics.waitAsync.paths)" \
        == "$(value config_wait_async_skip_paths)" \
    && "$(generated_value config.skip-feature:Atomics.waitAsync.variants)" \
        == "$(value config_wait_async_skip_variants)" ]] \
    || die 'SharedArrayBuffer universe partition counts drifted'

awk -F'\t' '!/^#/&&$1!="path"{print $1}' "$ledger" >"$tmp/universe.paths"
awk -F'\t' '!/^#/&&$1!="path"{print $1 "\t" $9}' "$ledger" \
    >"$tmp/universe-sources.tsv"
awk -F'\t' '!/^#/&&$1!="path"{n=split($3,v,",");for(i=1;i<=n;i++)print $1 "\t" v[i]}' \
    "$ledger" >"$tmp/universe.keys"
awk -F'\t' '!/^#/&&$1!="path"&&$2=="sab-core"{print $1 "\t" $9}' \
    "$ledger" >"$tmp/core-sources.tsv"
awk -F'\t' '!/^#/&&$1!="path"&&$2=="sab-core"{n=split($3,v,",");for(i=1;i<=n;i++)print $1 "\t" v[i]}' \
    "$ledger" >"$tmp/core.keys"
awk -F'\t' '!/^#/&&$1!="path"&&($2=="sab-core"||$2=="atomics-nonblocking"){print $1}' \
    "$ledger" >"$tmp/no-wait-no-agent.txt"
awk -F'\t' '!/^#/&&$1!="path"&&($2=="sab-core"||$2=="atomics-nonblocking"){print $1 "\t" $9}' \
    "$ledger" >"$tmp/no-wait-no-agent-sources.tsv"
awk -F'\t' '!/^#/&&$1!="path"&&($2=="sab-core"||$2=="atomics-nonblocking"){n=split($3,v,",");for(i=1;i<=n;i++)print $1 "\t" v[i]}' \
    "$ledger" >"$tmp/no-wait-no-agent.keys"

[[ "$(sha "$tmp/universe.paths")" == "$(value universe_paths_sha256)" \
    && "$(sha "$tmp/universe-sources.tsv")" == "$(value universe_source_projection_sha256)" \
    && "$(sha "$tmp/universe.keys")" == "$(value universe_keys_sha256)" \
    && "$(sha "$tmp/core-sources.tsv")" == "$(value core_source_projection_sha256)" \
    && "$(sha "$tmp/core.keys")" == "$(value core_keys_sha256)" \
    && "$(lines "$tmp/no-wait-no-agent.txt")" == "$(value no_wait_no_agent_paths)" \
    && "$(lines "$tmp/no-wait-no-agent.keys")" == "$(value no_wait_no_agent_variants)" \
    && "$(sha "$tmp/no-wait-no-agent.txt")" == "$(value no_wait_no_agent_paths_sha256)" \
    && "$(sha "$tmp/no-wait-no-agent-sources.tsv")" \
        == "$(value no_wait_no_agent_source_projection_sha256)" \
    && "$(sha "$tmp/no-wait-no-agent.keys")" == "$(value no_wait_no_agent_keys_sha256)" ]] \
    || die 'SharedArrayBuffer path, variant, or source projection drifted'

awk -F'\t' 'function has(list,value){return index("," list ",","," value ",")!=0}
!/^#/&&$1!="path"&&$2=="sab-core"{
    n=split($6,f,",");for(i=1;i<=n;i++)if(f[i]!="")seen[f[i]]=1
    if(has($7,"create-realm"))create_realm=1
} END{
    for(feature in seen)print feature
    if(create_realm)print "host-create-realm-required"
}' "$ledger" | sort >"$tmp/core.features"
diff -u <(section "$profile" features) "$tmp/core.features" \
    || die 'scoped profile is not the exact core feature union'

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
        || { cat "$rejection_log" >&2; die "unexpected selection rejection: $*"; }
}
expect_rejected_selection 'requires its pinned manifest' --all
expect_rejected_selection 'requires its pinned manifest' \
    --test test/built-ins/SharedArrayBuffer/length.js
expect_rejected_selection 'requires tests/test262-shared-array-buffer-core.txt' \
    --manifest "$tmp/no-wait-no-agent.txt"

focused_report=$tmp/focused.tsv
focused_json=${focused_report%.tsv}.jsonl
focused_log=$tmp/focused.log
"$runner" --suite "$suite" --config "$source_dir/test262.conf" \
    --oxide-profile "$profile" --manifest "$core" --report "$focused_report" \
    --mode both --timeout-ms 30000 --workers "$workers" --allow-failures \
    >"$focused_log" 2>&1 \
    || { cat "$focused_log" >&2; die 'exact core selection was not accepted'; }
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
        == "$(value core_variants)" \
    && "$(report_keys "$focused_report" | sha /dev/stdin)" \
        == "$(value core_keys_sha256)" \
    && "$(lines "$focused_report")" == "$(value oxide_focused_report_lines)" \
    && "$(lines "$focused_json")" == "$(value oxide_focused_jsonl_lines)" \
    && "$(sha "$focused_report")" == "$(value oxide_focused_tsv_sha256)" \
    && "$(sha "$focused_json")" == "$(value oxide_focused_jsonl_sha256)" \
    && "$(report_summary "$focused_report")" == "$(computed_summary "$focused_report")" \
    && "$(report_summary "$focused_report")" == "$(value oxide_focused_summary)" \
    && "$(report_rows "$focused_report" | awk -F'\t' '$7=="pass"{count++} END{print count+0}')" \
        == "$(value oxide_focused_passes)" ]] \
    || die 'exact core selection report identity drifted'
report_rows "$focused_report" | awk -F'\t' '$8=="selection"{exit 1}' \
    || die 'exact core profile left a row at the selection phase'

quickjs_runner=$source_dir/run-test262
[[ -x "$quickjs_runner" ]] || "${MAKE:-make}" -C "$source_dir" run-test262 >&2
files=()
while IFS= read -r test_path; do files+=("test262/$test_path"); done <"$core"
if ! (cd "$source_dir" && ./run-test262 -m -c test262.conf -a \
        -T "$workers" -f "${files[@]}") >"$oracle_log" 2>&1; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS failed the SharedArrayBuffer core frontier'
fi
if grep -Eq '(^|[[:space:]])FAILED($|[[:space:]])|SKIPPED FEATURE' "$oracle_log" \
    || [[ "$(grep -Fc 'Average memory statistics for 438 tests:' "$oracle_log")" != 1 ]]; then
    tail -n 100 "$oracle_log" >&2
    die 'pinned QuickJS SharedArrayBuffer core receipt drifted'
fi
{
    echo '# Pinned QuickJS oracle receipt for the R3dh SharedArrayBuffer core frontier.'
    echo "quickjs=$(value quickjs)"
    echo "test262=$(value test262)"
    echo "universe_ledger_sha256=$(value universe_ledger_sha256)"
    echo "core_manifest_sha256=$(value core_manifest_sha256)"
    echo "paths=$(value core_paths)"
    echo "variants=$(value core_variants)"
    echo 'failed=0'
    echo 'skipped_feature=0'
    echo 'result=pass'
} >"$tmp/quickjs-receipt.txt"
diff -u "$quickjs_receipt" "$tmp/quickjs-receipt.txt" \
    || die 'pinned QuickJS receipt projection drifted'

echo 'R3dh SharedArrayBuffer frontier verified: 463 paths / 922 variants partitioned; Oxide exact core 438 / 438; pinned QuickJS 438 / 438.'

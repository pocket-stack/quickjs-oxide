#!/usr/bin/env python3
"""Check the oracle module tree and, optionally, Cargo's compiled inventories."""
import argparse
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
ORACLE = ROOT / 'apps/cli/tests/oracle'
HOST_MODULES = {'test262_create_realm', 'test262_host_gc', 'test262_is_html_dda'}
MODULE = re.compile(r'^(?:pub(?:\([^)]*\))?\s+)?mod (\w+);$', re.MULTILINE)
TEST = re.compile(r'^\s*#\[test\]', re.MULTILINE)


def fail(message):
    raise SystemExit('error: ' + message)


def inventory():
    visited = set()
    host_modules = set()
    counts = {False: 0, True: 0}

    def visit(path, host=False):
        if path.is_symlink() or not path.is_file():
            fail(f'module must be a regular file: {path.relative_to(ROOT)}')
        path = path.resolve()
        if path in visited:
            fail(f'duplicate module registration: {path.relative_to(ROOT)}')
        if not path.is_relative_to(ROOT / 'apps/cli/tests') and path != ROOT / 'tests/common/mod.rs':
            fail(f'oracle module escapes tests/: {path}')
        visited.add(path)
        source = path.read_text()
        counts[host] += len(TEST.findall(source))
        for match in MODULE.finditer(source):
            name = match[1]
            prefix = source[:match.start()].splitlines()
            attributes = []
            while prefix and prefix[-1].startswith('#['):
                attributes.insert(0, prefix.pop())
            gates = [a for a in attributes if a.startswith('#[cfg')]
            gated = gates == ['#[cfg(feature = "test262-host")]']
            if gates and not gated:
                fail(f'unexpected module gate for {name}: {gates}')
            if name in HOST_MODULES:
                if path != ORACLE / 'main.rs' or not gated:
                    fail(f'{name} must be gated at the oracle entry point')
                host_modules.add(name)
            elif gated:
                fail(f'unexpected host-only module: {name}')
            paths = [a for a in attributes if a.startswith('#[path')]
            if paths:
                allowed = {
                    (ORACLE / 'main.rs', '#[path = "../common/mod.rs"]'): '../common/mod.rs',
                    (ROOT / 'apps/cli/tests/common/mod.rs', '#[path = "../../../../tests/common/mod.rs"]'): '../../../../tests/common/mod.rs',
                }
                relative = allowed.get((path, paths[0])) if len(paths) == 1 else None
                if relative is None:
                    fail(f'use ordinary module declarations for {name}')
                child = path.parent / relative
            else:
                directory = path.parent if path.name in ('main.rs', 'mod.rs') else path.with_suffix('')
                candidates = [directory / (name + '.rs'), directory / name / 'mod.rs']
                existing = [p for p in candidates if p.exists()]
                if len(existing) != 1:
                    fail(f'{name} must resolve to exactly one source file: {path.relative_to(ROOT)}')
                child = existing[0]
            visit(child, host or gated)

    visit(ORACLE / 'main.rs')
    if host_modules != HOST_MODULES:
        fail(f'host module inventory drifted: {sorted(host_modules)}')
    orphans = set(ORACLE.rglob('*.rs')) - visited
    if orphans:
        fail('unregistered oracle sources: ' + ', '.join(str(p.relative_to(ROOT)) for p in sorted(orphans)))
    if list((ROOT / 'apps/cli/tests').glob('oracle_*.rs')):
        fail('oracle sources must live under apps/cli/tests/oracle/')
    print(f'Oracle module tree covers {counts[False]} default + {counts[True]} host tests across {len(visited)} sources.')
    return counts


def compiled_inventory(features=False, test_filter=None):
    command = ['cargo', 'test', '--locked', '-p', 'quickjs-oxide-cli', '--test', 'oracle']
    if features:
        command += ['--features', 'test262-host']
    if test_filter:
        command.append(test_filter)
    result = subprocess.check_output(command + ['--', '--list'], cwd=ROOT, text=True)
    return {line.removesuffix(': test') for line in result.splitlines() if line.endswith(': test')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiled', action='store_true')
    args = parser.parse_args()
    counts = inventory()
    subprocess.run(['node', 'scripts/checks/check-oracle-helper-duplication.mjs'], cwd=ROOT, check=True)
    if args.compiled:
        default = compiled_inventory()
        host = compiled_inventory(True)
        filtered = compiled_inventory(True, 'test262_')
        if len(default) != counts[False] or len(host) != sum(counts.values()):
            fail('compiled test counts do not match registered source tests')
        if not default <= host or host - default != filtered or len(filtered) != counts[True]:
            fail('host feature/filter changes tests outside the registered host modules')
        print(f'Compiled oracle inventory matches: {len(default)} default / {len(host)} host / {len(filtered)} host-filtered tests.')


if __name__ == '__main__':
    main()

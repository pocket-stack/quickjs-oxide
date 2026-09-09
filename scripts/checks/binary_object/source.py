"""Source reading, Rust lexical masking, and structural evidence helpers."""
from __future__ import annotations

from pathlib import Path
import hashlib
import re

from .layout import owner_sources


# Skip ordinary Rust text in one search; only these prefixes need lexical work.
LEXICAL_PREFIX = re.compile(r'//|/\*|(?:br|rb|cr|rc|r)#{0,255}"|[bc]?"')


class SourceTools:
    def fail(ctx, code: str, message: str) -> None:
        ctx.errors.append(f"{code}: {message}")

    def read_source(ctx, relative: str) -> str:
        path = ctx.root / relative
        if path.is_symlink() or not path.is_file():
            ctx.fail("missing-source", f"{relative} must be a regular file")
            return ""
        source = path.read_text(encoding="utf-8")
        if relative == "src/engine/heap/runtime/tests.rs":
            # Expand only plain, ungated child declarations. Every evidence function
            # is still checked below, including its attributes and normalized body.
            def expand_test_module(match):
                child = f"src/engine/heap/runtime/tests/{match[1]}.rs"
                content = ctx.read_source(child)
                return re.sub(r"(?m)^use super::\*;\n", "", content)
            source = re.sub(r"(?m)^mod (\w+);$", expand_test_module, source)
        elif relative == "src/engine/code/binary_object/ordinary_leaf.rs":
            declaration = '#[cfg(test)]\nmod tests;'
            if declaration in source:
                child = ctx.read_source("src/engine/code/binary_object/ordinary_leaf/tests.rs")
                source = source.replace(declaration, '#[cfg(test)]\nmod tests {\n' + child + '\n}')
        if relative == "src/engine/vm/mod.rs":
            declaration = '#[cfg(test)]\nmod tests;'
            if declaration in source:
                child = ctx.read_source("src/engine/vm/tests.rs")
                source = source.replace(declaration, '#[cfg(test)]\nmod tests {\n' + child + '\n}')
        source += owner_sources(ctx, relative)
        return source

    def is_test_source(ctx, path: Path) -> bool:
        return path.name == "tests.rs" or "tests" in path.relative_to(ctx.root).parts

    def blank(ctx, text: str) -> str:
        return "".join('\n' if character == '\n' else " " for character in text)

    def rust_code_only(ctx, source: str) -> str:
        """Remove comments and strings while retaining offsets and line numbers."""
        output: list[str] = []
        index = 0
        length = len(source)
        while index < length:
            prefix = LEXICAL_PREFIX.search(source, index)
            if prefix is None:
                output.append(source[index:])
                break
            output.append(source[index:prefix.start()])
            index = prefix.start()
            if source.startswith("//", index):
                end = source.find('\n', index)
                if end < 0:
                    end = length
                output.append(ctx.blank(source[index:end]))
                index = end
                continue

            if source.startswith("/*", index):
                start = index
                depth = 1
                index += 2
                while index < length and depth:
                    if source.startswith("/*", index):
                        depth += 1
                        index += 2
                    elif source.startswith("*/", index):
                        depth -= 1
                        index += 2
                    else:
                        index += 1
                output.append(ctx.blank(source[start:index]))
                continue

            # Match against the original buffer at an absolute offset. Slicing the
            # entire remaining source at every byte turns large-file scans into an
            # accidental quadratic operation.
            raw = ctx.raw_string_prefix.match(source, index)
            if raw is not None:
                start = index
                hashes = raw.group("hashes")
                index = raw.end()
                terminator = '"' + hashes
                end = source.find(terminator, index)
                index = length if end < 0 else end + len(terminator)
                output.append(ctx.blank(source[start:index]))
                continue

            quote_offset = 1 if source[index:index + 2] in {'b"', 'c"'} else 0
            if source[index + quote_offset:index + quote_offset + 1] == '"':
                start = index
                index += quote_offset + 1
                while index < length:
                    if source[index] == "\\":
                        index = min(length, index + 2)
                    elif source[index] == '"':
                        index += 1
                        break
                    else:
                        index += 1
                output.append(ctx.blank(source[start:index]))
                continue

            output.append(source[index])
            index += 1

        return "".join(output)

    def location(ctx, relative: str, source: str, offset: int) -> str:
        line = source.count('\n', 0, offset) + 1
        line_start = source.rfind('\n', 0, offset) + 1
        line_end = source.find('\n', offset)
        if line_end < 0:
            line_end = len(source)
        excerpt = source[line_start:line_end].strip()
        return f"{relative}:{line}: {excerpt}"

    def normalized_code_sha256(ctx, code: str) -> str:
        return hashlib.sha256(" ".join(code.split()).encode("utf-8")).hexdigest()

    def require_normalized_code_sha256(ctx,
        code_name: str,
        description: str,
        code: str,
        expected: str,
    ) -> None:
        found = ctx.normalized_code_sha256(code)
        if found != expected:
            ctx.fail(code_name, f"{description}; normalized code sha256 {found}")

    def require_ordered_fragments(ctx,
        code_name: str,
        description: str,
        code: str,
        fragments: tuple[str, ...],
    ) -> None:
        normalized = " ".join(code.split())
        offsets = [normalized.find(fragment) for fragment in fragments]
        if (
            any(offset < 0 for offset in offsets)
            or offsets != sorted(offsets)
            or any(normalized.count(fragment) != 1 for fragment in fragments)
        ):
            ctx.fail(code_name, description)

    def require_normalized_corridor_sha256(ctx,
        code_name: str,
        description: str,
        code: str,
        start_fragment: str,
        end_fragment: str,
        expected: str,
    ) -> None:
        normalized = " ".join(code.split())
        if (
            normalized.count(start_fragment) != 1
            or normalized.count(end_fragment) != 1
        ):
            ctx.fail(code_name, description)
            return
        start = normalized.find(start_fragment)
        end_start = normalized.find(end_fragment, start)
        if end_start < start:
            ctx.fail(code_name, description)
            return
        corridor = normalized[start:end_start + len(end_fragment)]
        ctx.require_normalized_code_sha256(
            code_name,
            description,
            corridor,
            expected,
        )

    def unique_braced_item(ctx,
        code: str,
        pattern: re.Pattern[str],
        error_code: str,
        description: str,
    ) -> tuple[str, int, int]:
        matches = list(pattern.finditer(code))
        if len(matches) != 1:
            ctx.fail(error_code, f"must contain exactly one {description}")
            return "", -1, -1

        return ctx.braced_item_from_match(code, matches[0], error_code, description)

    def braced_item_from_match(ctx,
        code: str,
        item: re.Match[str],
        error_code: str,
        description: str,
    ) -> tuple[str, int, int]:
        depth = 0
        for offset in range(item.end() - 1, len(code)):
            if code[offset] == "{":
                depth += 1
            elif code[offset] == "}":
                depth -= 1
                if depth == 0:
                    return code[item.start():offset + 1], item.start(), offset + 1
        ctx.fail(error_code, f"{description} has no balanced closing brace")
        return "", -1, -1

    def rustfmt_match_arms(ctx, item: str, prefix: str) -> list[tuple[str, str]]:
        """Return normalized top-level arms from a rustfmt-indented match."""
        pattern = re.compile(
            rf"(?ms)^        (?P<lhs>{re.escape(prefix)}.*?) => (?P<rhs>.*?)"
            rf"(?=^        (?:{re.escape(prefix)}|_ =>)|^    \}})"
        )
        return [
            (
                " ".join(match.group("lhs").split()),
                " ".join(match.group("rhs").rstrip(', \n').split()),
            )
            for match in pattern.finditer(item)
        ]

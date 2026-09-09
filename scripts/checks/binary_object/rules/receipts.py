"""Receipts checks, in the ordered boundary scan."""
from __future__ import annotations

import hashlib
import html
import re
import unicodedata
from copy import deepcopy

from ..evidence import receipts as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    stage3e_status = ctx.read_source("docs/status.md")

    stage3j_non_ascii_status = tuple(
        (offset, character)
        for offset, character in enumerate(stage3e_status)
        if not character.isascii()
    )

    if stage3j_non_ascii_status:
        first_offset, first_character = stage3j_non_ascii_status[0]
        ctx.fail(
            "stage3j-status",
            "docs/status.md must remain ASCII so Stage3J/raw112 lifecycle, receipt, metric, and conformance claims cannot be hidden behind Unicode controls or confusables; found "
            + ctx.location(
                "docs/status.md",
                stage3e_status,
                first_offset,
            )
            + f" with U+{ord(first_character):04X}",
        )

    for ctx.description, start_fragment, end_fragment, ctx.expected_hash in (
        (
            "the status document must retain the exact explicit-throw typed path and no-VM-change boundary",
            "Stage 3D admits explicit raw 48",
            "already implemented exception path.",
            "9f1793c230bff05e3d1c8ea6e8e80b9dbe4ec64815a7fe684ffe4e58959e0f22",
        ),
        (
            "the status document must retain the exact Rust raw48 identity, backtrace, iterator-close, terminal, and rollback evidence",
            "Stage-3D Rust evidence uses",
            "transactional heap/atom rollback.",
            "8342c0f2e2b880cc8f0c668680db1521ca400a8cae2d837a261ed785fe6d3c09",
        ),
        (
            "the status document must retain the authenticated raw48 through raw112 C wires and current Stage3J source/transcript/manifest hashes",
            "Stage 3D adds the exact compiler-natural strict 45-byte raw-48 wire",
            "`e9f74aaa094cc4fb30b4a159d239d7e311622e55cd4c78f75723057a03569ee7`.",
            "7380c2f042c229c070f5b90151bb4c26c04bcaff02010eddebaeb5ca6599b895",
        ),
        (
            "the status document must retain the exact Stage3E typed atom/synthetic-constant path and no-new-VM boundary",
            "Stage 3E admits raw 49 only as the typed chain",
            "public API, source syntax, Test262 admission, or Feature Parity claim.",
            "f9772990f611da10923dded7873f94c581ae82c6c3cc5517daeb76e9b0fba341",
        ),
        (
            "the status document must retain the exact Stage3E natural/manual wire, atom provenance, terminal, realm, pending, catch, and rollback evidence",
            "Stage-3E Rust evidence distinguishes",
            "transactional retry after every rejected form.",
            "b52a229d39453fd9ef6abee7039e873cf14c9c42266e82c883850d7c612761c4",
        ),
    ):
        ctx.require_normalized_corridor_sha256(
            "stage3e-status",
            ctx.description,
            stage3e_status,
            start_fragment,
            end_fragment,
            ctx.expected_hash,
        )

    stage3f_status_corridors = deepcopy(evidence.STAGE3F_STATUS_CORRIDORS)

    stage3g_status_corridors = deepcopy(evidence.STAGE3G_STATUS_CORRIDORS)

    stage3h_status_corridors = deepcopy(evidence.STAGE3H_STATUS_CORRIDORS)

    stage3i_status_corridors = deepcopy(evidence.STAGE3I_STATUS_CORRIDORS)

    stage3j_status_corridors = deepcopy(evidence.STAGE3J_STATUS_CORRIDORS)

    stage3f_void_html_tags = {
        "area", "base", "br", "col", "embed", "hr", "img", "input",
        "link", "meta", "param", "source", "track", "wbr",
    }

    def stage3f_raw_html_ancestor_stack_at(
        source: str,
        stop: int,
    ) -> tuple[str, ...]:
        stack: list[str] = []
        index = 0
        while index < stop:
            opening = source.find("<", index, stop)
            if opening < 0:
                break
            if source.startswith("<!--", opening):
                comment_end = source.find("-->", opening + 4)
                if comment_end < 0 or comment_end + 3 > stop:
                    stack.append("!--")
                    break
                index = comment_end + 3
                continue

            cursor = opening + 1
            closing = cursor < stop and source[cursor] == "/"
            if closing:
                cursor += 1
            while cursor < stop and source[cursor].isspace():
                cursor += 1
            tag_match = re.match(r"[A-Za-z][A-Za-z0-9:-]*", source[cursor:stop])
            if tag_match is None:
                index = opening + 1
                continue
            tag = tag_match.group(0).lower()
            cursor += len(tag_match.group(0))
            quote: str | None = None
            tag_end = -1
            while cursor < len(source):
                character = source[cursor]
                if quote is not None:
                    if character == quote:
                        quote = None
                elif character in {"'", '"'}:
                    quote = character
                elif character == ">":
                    tag_end = cursor
                    break
                cursor += 1
            if tag_end < 0 or tag_end >= stop:
                stack.append("!incomplete-tag")
                break

            tag_tail = source[opening + 1:tag_end]
            if closing:
                if stack and stack[-1] == tag:
                    stack.pop()
                else:
                    stack.append(f"!mispaired:/{tag}")
            elif (
                tag not in stage3f_void_html_tags
                and not tag_tail.rstrip().endswith("/")
            ):
                stack.append(tag)
            index = tag_end + 1
        return tuple(stack)
    stage3f_raw_html_ancestor_stack_at = stage3f_raw_html_ancestor_stack_at

    def stage3f_active_fence_at(source: str, stop: int) -> tuple[str, int] | None:
        active: tuple[str, int] | None = None
        for line in source[:stop].splitlines():
            fence = re.match(
                r"^[ ]{0,3}(?P<marker>`{3,}|~{3,})(?P<tail>.*)$",
                line,
            )
            if fence is None:
                continue
            marker = fence.group("marker")
            tail = fence.group("tail")
            if active is None:
                if marker[0] != "`" or "`" not in tail:
                    active = (marker[0], len(marker))
            elif (
                marker[0] == active[0]
                and len(marker) >= active[1]
                and not tail.strip()
            ):
                active = None
        return active
    stage3f_active_fence_at = stage3f_active_fence_at

    def require_stage3f_top_level_corridor(
        diagnostic: str,
        description: str,
        start_fragment: str,
        end_fragment: str,
    ) -> None:
        normalized_characters: list[str] = []
        normalized_offsets: list[int] = []
        for token in re.finditer(r"\S+", stage3e_status):
            if normalized_characters:
                normalized_characters.append(" ")
                normalized_offsets.append(token.start())
            for offset in range(token.start(), token.end()):
                normalized_characters.append(stage3e_status[offset])
                normalized_offsets.append(offset)
        normalized_source = "".join(normalized_characters)
        if (
            normalized_source.count(start_fragment) != 1
            or normalized_source.count(end_fragment) != 1
        ):
            ctx.fail(diagnostic, description)
            return
        normalized_start = normalized_source.find(start_fragment)
        normalized_end = (
            normalized_source.find(end_fragment, normalized_start)
            + len(end_fragment)
        )
        start = normalized_offsets[normalized_start]
        end = normalized_offsets[normalized_end - 1] + 1
        line_start = stage3e_status.rfind('\n', 0, start) + 1
        line_end = stage3e_status.find('\n', end)
        if line_end < 0:
            line_end = len(stage3e_status)
        corridor_lines = stage3e_status[line_start:line_end].splitlines()
        indented = any(
            line and re.match(r"(?:[ ]{4,}|\t)", line)
            for line in corridor_lines
        )
        if (
            indented
            or stage3f_active_fence_at(stage3e_status, start) is not None
            or stage3f_active_fence_at(stage3e_status, end) is not None
            or stage3f_raw_html_ancestor_stack_at(stage3e_status, start)
            or stage3f_raw_html_ancestor_stack_at(stage3e_status, end)
        ):
            ctx.fail(
                diagnostic,
                f"{description}; the complete corridor must remain top-level rendered Markdown at both boundaries, not an indented/fenced block, comment, or raw-HTML descendant",
            )
    require_stage3f_top_level_corridor = require_stage3f_top_level_corridor

    for ctx.description, start_fragment, end_fragment, ctx.expected_hash in stage3f_status_corridors:
        ctx.require_normalized_corridor_sha256(
            "stage3f-status",
            ctx.description,
            stage3e_status,
            start_fragment,
            end_fragment,
            ctx.expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3f-status",
            ctx.description,
            start_fragment,
            end_fragment,
        )

    for ctx.description, start_fragment, end_fragment, ctx.expected_hash in stage3g_status_corridors:
        ctx.require_normalized_corridor_sha256(
            "stage3g-status",
            ctx.description,
            stage3e_status,
            start_fragment,
            end_fragment,
            ctx.expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3g-status",
            ctx.description,
            start_fragment,
            end_fragment,
        )

    for ctx.description, start_fragment, end_fragment, ctx.expected_hash in stage3h_status_corridors:
        ctx.require_normalized_corridor_sha256(
            "stage3h-status",
            ctx.description,
            stage3e_status,
            start_fragment,
            end_fragment,
            ctx.expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3h-status",
            ctx.description,
            start_fragment,
            end_fragment,
        )

    for ctx.description, start_fragment, end_fragment, ctx.expected_hash in stage3i_status_corridors:
        ctx.require_normalized_corridor_sha256(
            "stage3i-status",
            ctx.description,
            stage3e_status,
            start_fragment,
            end_fragment,
            ctx.expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3i-status",
            ctx.description,
            start_fragment,
            end_fragment,
        )

    for ctx.description, start_fragment, end_fragment, ctx.expected_hash in stage3j_status_corridors:
        ctx.require_normalized_corridor_sha256(
            "stage3j-status",
            ctx.description,
            stage3e_status,
            start_fragment,
            end_fragment,
            ctx.expected_hash,
        )
        require_stage3f_top_level_corridor(
            "stage3j-status",
            ctx.description,
            start_fragment,
            end_fragment,
        )

    stage3f_inline_stage_label_emphasis = re.compile(
        r"(?P<prefix>\bStage[- ]*3)"
        r"(?P<delimiter>\*{1,3}|_{1,3})"
        r"(?P<label>[HIJ])(?P=delimiter)(?![\w*_])",
        re.IGNORECASE,
    )

    def stage3f_strip_inline_stage_label_emphasis(source: str) -> str:
        return stage3f_inline_stage_label_emphasis.sub(
            lambda match: match.group("prefix") + match.group("label"),
            source,
        )
    stage3f_strip_inline_stage_label_emphasis = stage3f_strip_inline_stage_label_emphasis

    for delimiter in ("*", "**", "***", "_", "__", "___"):
        for ctx.label in ("H", "I", "J"):
            emphasized = f"Stage 3{delimiter}{ctx.label}{delimiter} lifecycle"
            ctx.expected = f"Stage 3{ctx.label} lifecycle"
            if stage3f_strip_inline_stage_label_emphasis(emphasized) != ctx.expected:
                ctx.fail(
                    f"stage3{ctx.label.lower()}-status",
                    "the bounded inline Stage 3H/3I/3J emphasis normalization must strip one to three matching Markdown emphasis markers",
                )

    for negative_control in (
        "Stage 30*H* lifecycle",
        "Stage 3*K* lifecycle",
        "Stage 3****H**** lifecycle",
        r"Stage 3\*H\* lifecycle",
        "Stage 3*H*idden lifecycle",
    ):
        if stage3f_strip_inline_stage_label_emphasis(negative_control) != negative_control:
            ctx.fail(
                "stage3h-status",
                "the bounded inline Stage 3H/3I/3J emphasis normalization must not consume non-label or escaped marker text",
            )

    def stage3f_rendered_text_projection(source: str) -> str:
        rendered: list[str] = []
        index = 0
        while index < len(source):
            if source.startswith("<!--", index):
                comment_end = source.find("-->", index + 4)
                index = len(source) if comment_end < 0 else comment_end + 3
                continue
            if source[index] == "<":
                cursor = index + 1
                if cursor < len(source) and source[cursor] == "/":
                    cursor += 1
                while cursor < len(source) and source[cursor].isspace():
                    cursor += 1
                tag_match = re.match(
                    r"[A-Za-z][A-Za-z0-9:-]*",
                    source[cursor:],
                )
                if tag_match is not None:
                    cursor += len(tag_match.group(0))
                    quote: str | None = None
                    while cursor < len(source):
                        character = source[cursor]
                        if quote is not None:
                            if character == quote:
                                quote = None
                        elif character in {"'", '"'}:
                            quote = character
                        elif character == ">":
                            index = cursor + 1
                            break
                        cursor += 1
                    else:
                        index = len(source)
                    continue
            rendered.append(source[index])
            index += 1

        projected = " ".join(html.unescape("".join(rendered)).split())
        projected = stage3f_strip_inline_stage_label_emphasis(projected)
        projected = re.sub(
            r"(?<![\w\\])(?:\*{1,3}|_{1,3})(?=\S)",
            "",
            projected,
        )
        projected = re.sub(
            r"(?<=\S)(?:\*{1,3}|_{1,3})(?!\w)",
            "",
            projected,
        )
        return " ".join(projected.split())
    stage3f_rendered_text_projection = stage3f_rendered_text_projection

    def stage3f_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3F\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?177\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )
    stage3f_selected_status_sentences = stage3f_selected_status_sentences

    def stage3g_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3G\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?11\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )
    stage3g_selected_status_sentences = stage3g_selected_status_sentences

    def stage3h_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3H\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?111\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact|fingerprint|reports?)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )
    stage3h_selected_status_sentences = stage3h_selected_status_sentences

    def stage3i_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and re.search(r"\bStage[- ]*3I\b", sentence, re.IGNORECASE)
        )
    stage3i_selected_status_sentences = stage3i_selected_status_sentences

    def stage3j_selected_status_sentences(source: str) -> tuple[str, ...]:
        return tuple(
            sentence.strip()
            for sentence in re.split(r"(?<=[.!?])[ \t]+", source)
            if sentence.strip()
            and (
                re.search(r"\bStage[- ]*3J\b", sentence, re.IGNORECASE)
                or (
                    re.search(r"\braw[- ]?112\b", sentence, re.IGNORECASE)
                    and re.search(
                        r"\b(?:receipt|run|artifact|fingerprint|reports?)\b",
                        sentence,
                        re.IGNORECASE,
                    )
                )
            )
        )
    stage3j_selected_status_sentences = stage3j_selected_status_sentences

    stage3f_default_ignorable_ranges = deepcopy(evidence.STAGE3F_DEFAULT_IGNORABLE_RANGES)

    def stage3f_is_default_ignorable(character: str) -> bool:
        if unicodedata.category(character) == "Cf":
            return True
        codepoint = ord(character)
        return any(
            start <= codepoint <= end
            for start, end in stage3f_default_ignorable_ranges
        )
    stage3f_is_default_ignorable = stage3f_is_default_ignorable

    def stage3f_html_attributes(
        attributes: str,
    ) -> list[tuple[str, str | None]]:
        parsed: list[tuple[str, str | None]] = []
        index = 0
        while index < len(attributes):
            while (
                index < len(attributes)
                and (attributes[index].isspace() or attributes[index] == "/")
            ):
                index += 1
            if index >= len(attributes):
                break

            name_start = index
            while (
                index < len(attributes)
                and not attributes[index].isspace()
                and attributes[index] not in {'"', "'", "<", ">", "/", "="}
            ):
                index += 1
            if name_start == index:
                index += 1
                continue
            name = attributes[name_start:index].casefold()

            while index < len(attributes) and attributes[index].isspace():
                index += 1
            value: str | None = None
            if index < len(attributes) and attributes[index] == "=":
                index += 1
                while index < len(attributes) and attributes[index].isspace():
                    index += 1
                if index < len(attributes) and attributes[index] in {'"', "'"}:
                    quote = attributes[index]
                    index += 1
                    value_start = index
                    while index < len(attributes) and attributes[index] != quote:
                        index += 1
                    value = attributes[value_start:index]
                    if index < len(attributes):
                        index += 1
                else:
                    value_start = index
                    while (
                        index < len(attributes)
                        and not attributes[index].isspace()
                        and attributes[index] != ">"
                    ):
                        index += 1
                    value = attributes[value_start:index]
            parsed.append((name, value))
        return parsed
    stage3f_html_attributes = stage3f_html_attributes

    def stage3f_reference_label(label: str) -> str:
        return " ".join(label.split()).casefold()
    stage3f_reference_label = stage3f_reference_label

    def stage3f_forensic_text_projection(source: str) -> str:
        comment_content: list[str] = []
        index = 0
        while index < len(source):
            if source.startswith("<!--", index):
                comment_end = source.find("-->", index + 4)
                if comment_end < 0:
                    comment_content.append(source[index + 4:])
                    break
                comment_content.append(source[index + 4:comment_end])
                index = comment_end + 3
                continue
            comment_content.append(source[index])
            index += 1

        without_comments = "".join(comment_content)
        rendered: list[str] = []
        index = 0
        while index < len(without_comments):
            if without_comments[index] == "<":
                cursor = index + 1
                closing_tag = (
                    cursor < len(without_comments)
                    and without_comments[cursor] == "/"
                )
                if closing_tag:
                    cursor += 1
                while (
                    cursor < len(without_comments)
                    and without_comments[cursor].isspace()
                ):
                    cursor += 1
                tag_match = re.match(
                    r"[A-Za-z][A-Za-z0-9:-]*",
                    without_comments[cursor:],
                )
                if tag_match is not None:
                    tag_name = tag_match.group(0).lower()
                    cursor += len(tag_match.group(0))
                    attributes_start = cursor
                    quote: str | None = None
                    while cursor < len(without_comments):
                        character = without_comments[cursor]
                        if quote is not None:
                            if character == quote:
                                quote = None
                        elif character in {"'", '"'}:
                            quote = character
                        elif character == ">":
                            if tag_name == "img" and not closing_tag:
                                attributes = without_comments[
                                    attributes_start:cursor
                                ]
                                for name, value in stage3f_html_attributes(
                                    attributes
                                ):
                                    if name == "alt":
                                        rendered.append(value or "")
                                        break
                            index = cursor + 1
                            break
                        cursor += 1
                    else:
                        index = len(without_comments)
                    continue
            rendered.append(without_comments[index])
            index += 1

        projected = unicodedata.normalize(
            "NFKC",
            html.unescape("".join(rendered)),
        )
        projected = "".join(
            character
            for character in projected
            if not stage3f_is_default_ignorable(character)
        )
        reference_definitions = {
            stage3f_reference_label(match.group("label"))
            for match in re.finditer(
                r"(?m)^[ \t]{0,3}\[(?P<label>[^\]\n]+)\]:"
                r"[ \t]*(?:<[^>\n]+>|\S+)",
                projected,
            )
        }
        projected = " ".join(projected.split())

        code_span = re.compile(
            r"(?P<ticks>`+)(?P<content>.*?)(?P=ticks)",
        )
        projected = code_span.sub(
            lambda match: match.group("content"),
            projected,
        )
        projected = re.sub(
            r"!\[([^]\n]*)\]\([^)]*\)",
            r"\1",
            projected,
        )
        projected = re.sub(
            r"!\[([^]\n]*)\]\[[^]\n]*\]",
            r"\1",
            projected,
        )
        projected = re.sub(
            r"!\[(?P<label>[^]\n]+)\](?![\[(])",
            lambda match: (
                match.group("label")
                if stage3f_reference_label(match.group("label"))
                in reference_definitions
                else match.group(0)
            ),
            projected,
        )
        projected = re.sub(
            r"\[([^]\n]+)\]\([^)]*\)",
            r"\1",
            projected,
        )
        projected = re.sub(
            r"\[([^]\n]+)\]\[[^]\n]*\]",
            r"\1",
            projected,
        )
        projected = projected.replace("~~", "")
        projected = stage3f_strip_inline_stage_label_emphasis(projected)
        projected = re.sub(
            r"(?<![\w\\])(?:\*{1,3}|_{1,3})(?=\S)",
            "",
            projected,
        )
        projected = re.sub(
            r"(?<=\S)(?:\*{1,3}|_{1,3})(?!\w)",
            "",
            projected,
        )
        return " ".join(projected.split())
    stage3f_forensic_text_projection = stage3f_forensic_text_projection

    normalized_stage3f_status = " ".join(stage3e_status.split())

    rendered_stage3f_status = stage3f_rendered_text_projection(stage3e_status)

    forensic_stage3f_status = stage3f_forensic_text_projection(stage3e_status)

    stage3f_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )

    stage3f_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3F\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )

    if stage3f_source_current_negation.search(forensic_stage3f_status):
        ctx.fail(
            "stage3f-status",
            "the promoted Stage3F source-current status must not be negated",
        )

    if stage3f_stale_or_pending_claim.search(forensic_stage3f_status):
        ctx.fail(
            "stage3f-status",
            "the promoted Stage3F receipt must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3f_status_sentences = stage3f_selected_status_sentences(
        normalized_stage3f_status
    )

    stage3f_status_sentence_hash = hashlib.sha256(
        '\n'.join(stage3f_status_sentences).encode("utf-8")
    ).hexdigest()

    if (
        len(stage3f_status_sentences) != 7
        or stage3f_status_sentence_hash
        != "941d8c5cace97d5c9b19f37bd1c55577c7debf6c7a864f56b5809c40bb3d88c0"
    ):
        ctx.fail(
            "stage3f-status",
            "the ordered canonical Stage3F status/provenance sentence inventory must remain exact; "
            f"found {len(stage3f_status_sentences)} sentences with sha256 {stage3f_status_sentence_hash}",
        )

    rendered_stage3f_status_sentences = stage3f_selected_status_sentences(
        rendered_stage3f_status
    )

    rendered_stage3f_status_sentence_hash = hashlib.sha256(
        '\n'.join(rendered_stage3f_status_sentences).encode("utf-8")
    ).hexdigest()

    if (
        len(rendered_stage3f_status_sentences) != 7
        or rendered_stage3f_status_sentence_hash
        != "941d8c5cace97d5c9b19f37bd1c55577c7debf6c7a864f56b5809c40bb3d88c0"
    ):
        ctx.fail(
            "stage3f-status",
            "the ordered rendered-text Stage3F/status-provenance sentence inventory must remain exact after stripping HTML comments/tags and Markdown emphasis, decoding entities, and normalizing Unicode whitespace; "
            f"found {len(rendered_stage3f_status_sentences)} sentences with sha256 {rendered_stage3f_status_sentence_hash}",
        )

    forensic_stage3f_status_sentences = stage3f_selected_status_sentences(
        forensic_stage3f_status
    )

    forensic_stage3f_status_sentence_hash = hashlib.sha256(
        '\n'.join(forensic_stage3f_status_sentences).encode("utf-8")
    ).hexdigest()

    if (
        len(forensic_stage3f_status_sentences) != 7
        or forensic_stage3f_status_sentence_hash
        != "26d86780d083b9713d3f50c871943471ec704d2e6a6652c3e3281c0eda7c3a22"
    ):
        ctx.fail(
            "stage3f-status",
            "the ordered forensic Stage3F/status-provenance sentence inventory must remain exact after retaining HTML comment content, stripping tags while preserving image alt text, decoding entities, NFKC normalization, removing default-ignorable formatting, flattening Markdown code/link/image labels, and stripping emphasis/strong/strikethrough delimiters; "
            f"found {len(forensic_stage3f_status_sentences)} sentences with sha256 {forensic_stage3f_status_sentence_hash}",
        )

    stage3g_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )

    stage3g_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3G\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )

    if stage3g_source_current_negation.search(forensic_stage3f_status):
        ctx.fail(
            "stage3g-status",
            "the promoted Stage3G source-current status must not be negated",
        )

    if stage3g_stale_or_pending_claim.search(forensic_stage3f_status):
        ctx.fail(
            "stage3g-status",
            "the promoted Stage3G receipt must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3g_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            7,
            "613ef31592fa44fb2d94e9bfe4ed39bdc4ed23fcaccd0bb43756532319d7fb84",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            7,
            "613ef31592fa44fb2d94e9bfe4ed39bdc4ed23fcaccd0bb43756532319d7fb84",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            7,
            "2343cc10be9246638a622b589c9882ba963d9ce595d47ea82253c68003a89002",
        ),
    )

    for projection_name, projection, expected_count, ctx.expected_hash in stage3g_status_projections:
        sentences = stage3g_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            '\n'.join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != ctx.expected_hash:
            ctx.fail(
                "stage3g-status",
                "the ordered Stage3G/raw11 status sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3h_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            8,
            "c254cbe69a1d821919a6ab13b45e529cdf906bafe050968531f78d9ac551f78b",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            8,
            "c254cbe69a1d821919a6ab13b45e529cdf906bafe050968531f78d9ac551f78b",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            8,
            "6d3be83d309930bd072902bc93e395bdaa3c59bb711838481368b7090963fbd6",
        ),
        (
            "forensic-confusable-skeleton",
            "".join(
                character
                for character in forensic_stage3f_status.translate(
                    str.maketrans(
                        {
                            "\u041d": "H",
                            "\u043d": "h",
                            "\u0397": "H",
                            "\u03b7": "h",
                        }
                    )
                )
                if unicodedata.category(character) not in {"Cc", "Cf"}
            ),
            8,
            "6d3be83d309930bd072902bc93e395bdaa3c59bb711838481368b7090963fbd6",
        ),
    )

    for projection_name, projection, expected_count, ctx.expected_hash in stage3h_status_projections:
        sentences = stage3h_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            '\n'.join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != ctx.expected_hash:
            ctx.fail(
                "stage3h-status",
                "the ordered Stage3H/raw111 status sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3h_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )

    stage3h_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3H\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )

    stage3h_forensic_claim_projections = (
        forensic_stage3f_status,
        stage3h_status_projections[-1][1],
    )

    if any(
        stage3h_source_current_negation.search(projection)
        for projection in stage3h_forensic_claim_projections
    ):
        ctx.fail(
            "stage3h-status",
            "the promoted Stage3H source-current status must not be negated",
        )

    if any(
        stage3h_stale_or_pending_claim.search(projection)
        for projection in stage3h_forensic_claim_projections
    ):
        ctx.fail(
            "stage3h-status",
            "the promoted receipt's inherited Stage3H coverage must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3i_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            13,
            "16e0314decaad0477464a99f4221009b8e14742d859708073fd8cccfdbad16fe",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            13,
            "16e0314decaad0477464a99f4221009b8e14742d859708073fd8cccfdbad16fe",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            13,
            "9389a273e4982a760f662d39247142f6ea5a2aa206954faaf3f88be818944a55",
        ),
        (
            "forensic-confusable-skeleton",
            "".join(
                character
                for character in forensic_stage3f_status.translate(
                    str.maketrans(
                        {
                            "\u0406": "I",
                            "\u0456": "i",
                            "\u0399": "I",
                            "\u03b9": "i",
                        }
                    )
                )
                if unicodedata.category(character) not in {"Cc", "Cf"}
            ),
            13,
            "9389a273e4982a760f662d39247142f6ea5a2aa206954faaf3f88be818944a55",
        ),
    )

    for projection_name, projection, expected_count, ctx.expected_hash in stage3i_status_projections:
        sentences = stage3i_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            '\n'.join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != ctx.expected_hash:
            ctx.fail(
                "stage3i-status",
                "the ordered Stage3I/raw8 lifecycle sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3i_source_current_negation = re.compile(
        r"\b(?:not|never|no[ \t]+longer)\b[^.;!?]{0,32}\bsource[- ]current\b",
        re.IGNORECASE,
    )

    stage3i_stale_or_pending_claim = re.compile(
        r"\bStage[- ]*3I\b[^.;!?]{0,100}\b(?:source[- ]ahead|source[- ]stale|"
        r"unauthenticated|uncertified|pending|awaiting)\b",
        re.IGNORECASE,
    )

    stage3i_forensic_claim_projections = (
        forensic_stage3f_status,
        stage3i_status_projections[-1][1],
    )

    if any(
        stage3i_source_current_negation.search(projection)
        for projection in stage3i_forensic_claim_projections
    ):
        ctx.fail(
            "stage3i-status",
            "the promoted Stage3I source-current status must not be negated",
        )

    if any(
        stage3i_stale_or_pending_claim.search(projection)
        for projection in stage3i_forensic_claim_projections
    ):
        ctx.fail(
            "stage3i-status",
            "the promoted Stage3I receipt must not be contradicted by a source-ahead, stale, unauthenticated, uncertified, pending, or awaiting claim",
        )

    stage3j_status_projections = (
        (
            "canonical",
            normalized_stage3f_status,
            8,
            "d3121a9039e786b4277bd2e83ad9d1961ba05b37caa48918491bae3582f11f15",
        ),
        (
            "rendered-text",
            rendered_stage3f_status,
            8,
            "d3121a9039e786b4277bd2e83ad9d1961ba05b37caa48918491bae3582f11f15",
        ),
        (
            "forensic",
            forensic_stage3f_status,
            8,
            "cdaf528ca58a4112185202ca1b0aa4adfe104ca5efe0e6f0c4364622a1ec72fc",
        ),
        (
            "forensic-confusable-skeleton",
            "".join(
                character
                for character in forensic_stage3f_status.translate(
                    str.maketrans(
                        {
                            "\u0408": "J",
                            "\u0458": "j",
                        }
                    )
                )
                if unicodedata.category(character) not in {"Cc", "Cf"}
            ),
            8,
            "cdaf528ca58a4112185202ca1b0aa4adfe104ca5efe0e6f0c4364622a1ec72fc",
        ),
    )

    for projection_name, projection, expected_count, ctx.expected_hash in stage3j_status_projections:
        sentences = stage3j_selected_status_sentences(projection)
        sentence_hash = hashlib.sha256(
            '\n'.join(sentences).encode("utf-8")
        ).hexdigest()
        if len(sentences) != expected_count or sentence_hash != ctx.expected_hash:
            ctx.fail(
                "stage3j-status",
                "the ordered Stage3J/raw112 source-ahead lifecycle sentence inventory must remain exact in the "
                f"{projection_name} projection; found {len(sentences)} sentences with sha256 {sentence_hash}",
            )

    stage3j_contrary_claims = deepcopy(evidence.STAGE3J_CONTRARY_CLAIMS)

    stage3j_forensic_claim_projections = (
        forensic_stage3f_status,
        stage3j_status_projections[-1][1],
    )

    stage3j_required_claims = deepcopy(evidence.STAGE3J_REQUIRED_CLAIMS)

    if any(
        not re.search(pattern, projection, re.IGNORECASE)
        for projection in stage3j_forensic_claim_projections
        for pattern in stage3j_required_claims
    ):
        ctx.fail(
            "stage3j-status",
            "Stage3J/raw112 must remain explicitly source-ahead of the promoted Stage3I receipt, not covered or authenticated by it, without a promoted Stage3J receipt, metric change, or new conformance claim",
        )

    if any(
        re.search(pattern, projection, re.IGNORECASE)
        for projection in stage3j_forensic_claim_projections
        for pattern in stage3j_contrary_claims
    ):
        ctx.fail(
            "stage3j-status",
            "Stage3J/raw112 source-current, receipt-covered, authenticated, certified, source-ahead-negating, or new-conformance claims are forbidden while Stage3J remains source-ahead of the promoted Stage3I receipt",
        )

    stage3e_receipt_paragraphs = [
        paragraph
        for paragraph in re.split(r"\n[ \t]*\n", stage3e_status)
        if "The latest promoted Stage 3I lifecycle receipt" in paragraph
    ]

    stage3e_receipt_config = ctx.read_source("dev-support/test262/current.conf")

    stage3e_receipt_values: dict[str, str] = {}

    for line_number, line in enumerate(stage3e_receipt_config.splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if (
            separator != "="
            or not re.fullmatch(r"[a-z][a-z0-9_]*", key)
            or not value
        ):
            ctx.fail(
                "stage3i-status",
                f"dev-support/test262/current.conf:{line_number} is not a canonical key=value receipt field",
            )
            continue
        if key in stage3e_receipt_values:
            ctx.fail(
                "stage3i-status",
                f"dev-support/test262/current.conf repeats receipt field {key}",
            )
            continue
        stage3e_receipt_values[key] = value

    stage3e_receipt_metrics = deepcopy(evidence.STAGE3E_RECEIPT_METRICS)

    drifted_stage3e_receipt_metrics = {
        key: stage3e_receipt_values.get(key)
        for key, expected in stage3e_receipt_metrics.items()
        if stage3e_receipt_values.get(key) != expected
    }

    if drifted_stage3e_receipt_metrics:
        ctx.fail(
            "stage3i-status",
            "the promoted Stage3I receipt must retain the exact unchanged R3fj focused/full metrics; "
            f"found {drifted_stage3e_receipt_metrics}",
        )

    stage3e_receipt_hex_fields = deepcopy(evidence.STAGE3E_RECEIPT_HEX_FIELDS)

    invalid_stage3e_receipt_fields = {
        key: stage3e_receipt_values.get(key)
        for key, width in stage3e_receipt_hex_fields.items()
        if not re.fullmatch(
            rf"[0-9a-f]{{{width}}}", stage3e_receipt_values.get(key, "")
        )
    }

    if invalid_stage3e_receipt_fields:
        ctx.fail(
            "stage3i-status",
            "the promoted Stage3I receipt source, fingerprint, and focused/full report hashes must be canonical current.conf fields; "
            f"found {invalid_stage3e_receipt_fields}",
        )

    stage3e_promoted_receipt_values = deepcopy(evidence.STAGE3E_PROMOTED_RECEIPT_VALUES)

    drifted_stage3e_promoted_receipt_values = {
        key: stage3e_receipt_values.get(key)
        for key, expected in stage3e_promoted_receipt_values.items()
        if stage3e_receipt_values.get(key) != expected
    }

    if drifted_stage3e_promoted_receipt_values:
        ctx.fail(
            "stage3i-receipt-pin",
            "current.conf must retain the exact reviewed Stage3I source, fingerprint, and focused/full receipt hashes; "
            f"found {drifted_stage3e_promoted_receipt_values}",
        )

    stage3e_focused_receipt_paths = deepcopy(evidence.STAGE3E_FOCUSED_RECEIPT_PATHS)

    resolved_root = ctx.root.resolve()

    for path_key, (expected_relative, hash_key, expected_bytes) in stage3e_focused_receipt_paths.items():
        configured_relative = stage3e_receipt_values.get(path_key)
        if configured_relative != expected_relative:
            ctx.fail(
                "stage3i-focused-receipt",
                f"current.conf {path_key} must resolve the exact reviewed path {expected_relative}; found {configured_relative!r}",
            )
            continue
        candidate = ctx.root / configured_relative
        expected_lexical_path = resolved_root / expected_relative
        try:
            resolved_candidate = candidate.resolve(strict=True)
        except (OSError, RuntimeError) as error:
            ctx.fail(
                "stage3i-focused-receipt",
                f"{expected_relative} must resolve as the reviewed focused receipt: {error}",
            )
            continue
        if (
            candidate.is_symlink()
            or not candidate.is_file()
            or resolved_candidate != expected_lexical_path
            or candidate.stat().st_nlink != 1
            or candidate.stat().st_size != expected_bytes
        ):
            ctx.fail(
                "stage3i-focused-receipt",
                f"{expected_relative} must be the exact {expected_bytes}-byte regular, non-symlink, single-link reviewed focused receipt",
            )
            continue
        actual_hash = hashlib.sha256(candidate.read_bytes()).hexdigest()
        configured_hash = stage3e_receipt_values.get(hash_key)
        if actual_hash != configured_hash:
            ctx.fail(
                "stage3i-focused-receipt",
                f"{expected_relative} bytes must match current.conf {hash_key}; found {actual_hash}",
            )

    if len(stage3e_receipt_paragraphs) != 1:
        ctx.fail(
            "stage3i-status",
            "the status document must contain exactly one promoted Stage3I R3fj receipt paragraph with a blank-line boundary",
        )
    elif not invalid_stage3e_receipt_fields and not drifted_stage3e_receipt_metrics:
        stage3e_receipt_offset = stage3e_status.find(stage3e_receipt_paragraphs[0])
        stage3e_receipt_prefix = stage3e_status[:stage3e_receipt_offset]
        stage3e_receipt_is_indented_code = any(
            re.match(r"(?:[ ]{4,}|\t)", line)
            for line in stage3e_receipt_paragraphs[0].splitlines()
            if line
        )
        active_stage3e_fence: tuple[str, int] | None = None
        for line in stage3e_receipt_prefix.splitlines():
            fence = re.match(
                r"^[ ]{0,3}(?P<marker>`{3,}|~{3,})(?P<tail>.*)$", line
            )
            if fence is None:
                continue
            marker = fence.group("marker")
            tail = fence.group("tail")
            if active_stage3e_fence is None:
                if marker[0] != "`" or "`" not in tail:
                    active_stage3e_fence = (marker[0], len(marker))
            elif (
                marker[0] == active_stage3e_fence[0]
                and len(marker) >= active_stage3e_fence[1]
                and not tail.strip()
            ):
                active_stage3e_fence = None

        stage3e_void_html_tags = {
            "area", "base", "br", "col", "embed", "hr", "img", "input",
            "link", "meta", "param", "source", "track", "wbr",
        }

        def raw_html_ancestor_stack_at(source: str, stop: int) -> tuple[str, ...]:
            stack: list[str] = []
            index = 0
            while index < stop:
                opening = source.find("<", index, stop)
                if opening < 0:
                    break
                if source.startswith("<!--", opening):
                    comment_end = source.find("-->", opening + 4)
                    if comment_end < 0 or comment_end + 3 > stop:
                        stack.append("!--")
                        break
                    index = comment_end + 3
                    continue

                cursor = opening + 1
                closing = cursor < stop and source[cursor] == "/"
                if closing:
                    cursor += 1
                while cursor < stop and source[cursor].isspace():
                    cursor += 1
                tag_match = re.match(r"[A-Za-z][A-Za-z0-9:-]*", source[cursor:stop])
                if tag_match is None:
                    index = opening + 1
                    continue
                tag = tag_match.group(0).lower()
                cursor += len(tag_match.group(0))
                quote: str | None = None
                tag_end = -1
                while cursor < len(source):
                    character = source[cursor]
                    if quote is not None:
                        if character == quote:
                            quote = None
                    elif character in {"'", '"'}:
                        quote = character
                    elif character == ">":
                        tag_end = cursor
                        break
                    cursor += 1
                if tag_end < 0 or tag_end >= stop:
                    stack.append("!incomplete-tag")
                    break

                tag_tail = source[opening + 1:tag_end]
                if closing:
                    if stack and stack[-1] == tag:
                        stack.pop()
                    else:
                        stack.append(f"!mispaired:/{tag}")
                elif (
                    tag not in stage3e_void_html_tags
                    and not tag_tail.rstrip().endswith("/")
                ):
                    stack.append(tag)
                index = tag_end + 1
            return tuple(stack)
        raw_html_ancestor_stack_at = raw_html_ancestor_stack_at

        stage3e_receipt_end = (
            stage3e_receipt_offset + len(stage3e_receipt_paragraphs[0])
        )
        stage3e_receipt_html_at_start = raw_html_ancestor_stack_at(
            stage3e_status,
            stage3e_receipt_offset,
        )
        stage3e_receipt_html_at_end = raw_html_ancestor_stack_at(
            stage3e_status,
            stage3e_receipt_end,
        )
        if (
            stage3e_receipt_is_indented_code
            or active_stage3e_fence is not None
            or stage3e_receipt_html_at_start
            or stage3e_receipt_html_at_end
        ):
            ctx.fail(
                "stage3i-status",
                "the promoted Stage3I R3fj receipt paragraph must remain top-level rendered Markdown, not a comment, code block, or raw-HTML descendant at either boundary",
            )

        stage3e_receipt_provenance = {
            "promotion_commit": "a8a746f50dfc71ed560df32af9da2fb5488f7cba",
            "run": "32517600968",
            "job": "96882617930",
            "artifact": "9460012228",
            "artifact_sha256": "06a95f6510ad335bd090be5bb34168e8bf683e36336441b67ebb4a831f56fd9f",
            "stage3h_fingerprint": "d21943622773d2b0b978cd2ace5261d5ec41a9400ab36864768470aae71b1d22",
            "stage3h_run": "32419997996",
            "stage3h_artifact": "9425844939",
            "stage3h_artifact_sha256": "2c8dc920428aef4f10be440d7d18fdf72ec0902af4c7482f5be34ed4f25b1215",
        }
        ctx.source = re.escape(stage3e_receipt_values["engine_semantics_source"])
        fingerprint = re.escape(stage3e_receipt_values["engine_semantics_sha256"])
        focused_tsv_hash = re.escape(stage3e_receipt_values["focused_tsv_sha256"])
        focused_jsonl_hash = re.escape(stage3e_receipt_values["focused_jsonl_sha256"])
        full_tsv_hash = re.escape(stage3e_receipt_values["full_tsv_sha256"])
        full_jsonl_hash = re.escape(stage3e_receipt_values["full_jsonl_sha256"])
        focused_passes = f'{int(stage3e_receipt_values["focused_passes"]):,}'
        full_variants = f'{int(stage3e_receipt_values["full_variants"]):,}'
        full_eligible = f'{int(stage3e_receipt_values["full_eligible"]):,}'
        full_passes = f'{int(stage3e_receipt_values["full_passes"]):,}'
        full_tsv_lines = f'{int(stage3e_receipt_values["full_tsv_lines"]):,}'
        full_jsonl_lines = f'{int(stage3e_receipt_values["full_jsonl_lines"]):,}'
        receipt_pattern = re.compile(
            rf"The latest promoted Stage 3I lifecycle receipt is exact-source GitHub Actions "
            rf"run `{stage3e_receipt_provenance['run']}`, job `{stage3e_receipt_provenance['job']}`, "
            rf"on receipt-promotion commit `{stage3e_receipt_provenance['promotion_commit']}`\. "
            rf"It authenticates the Stage 3I engine-semantics source `{ctx.source}` with engine fingerprint `{fingerprint}`\. "
            rf"The canonical `test262-receipt` run completed successfully; its "
            rf"unique exact six-file artifact `{stage3e_receipt_provenance['artifact']}` \(SHA-256 "
            rf"`{stage3e_receipt_provenance['artifact_sha256']}`\) records the "
            rf"{full_tsv_lines}-line TSV as `{full_tsv_hash}` and the {full_jsonl_lines}-line JSONL as "
            rf"`{full_jsonl_hash}`\. Each full receipt contains the Stage 3I fingerprint exactly once and no Stage "
            rf"3H fingerprint\. Fingerprint-only normalization replaces that single occurrence "
            rf"with the authenticated Stage 3H fingerprint `{stage3e_receipt_provenance['stage3h_fingerprint']}` "
            rf"and makes both files byte-for-byte "
            rf"identical to Stage 3H run `{stage3e_receipt_provenance['stage3h_run']}`, "
            rf"artifact `{stage3e_receipt_provenance['stage3h_artifact']}` \(SHA-256 "
            rf"`{stage3e_receipt_provenance['stage3h_artifact_sha256']}`\): "
            rf"all {full_variants} classified outcomes, {full_passes} full passes, and {full_eligible} eligible variants "
            rf"are unchanged\. The refreshed {focused_passes}-pass focused TSV and JSONL are "
            rf"byte-identical on exact-source replay at hashes `{focused_tsv_hash}` and `{focused_jsonl_hash}`\. "
            rf"This promoted receipt is source-current for Stage 3I and covers the raw-8 "
            rf"`PushThis` admission and its Rust/C evidence without changing the Test262 "
            rf"profile or any focused or full metric reported above\. It remains the exact "
            rf"Stage-3I lifecycle boundary, retains the Stage-3H raw-111 `ToObject`, Stage-3G "
            rf"raw-11 Object, and Stage-3F "
            rf"raw-177 coverage, and makes no new conformance claim\."
        )
        normalized_receipt_paragraph = " ".join(
            stage3e_receipt_paragraphs[0].split()
        )
        if receipt_pattern.fullmatch(normalized_receipt_paragraph) is None:
            ctx.fail(
                "stage3i-status",
                "the complete promoted Stage3I R3fj paragraph must match current.conf source, fingerprint, focused/full report hashes, exact successful provenance, normalized Stage3H equality, inherited Stage3H/Stage3G/Stage3F coverage, unchanged metrics, and its blank-line boundary",
            )

    normalized_stage3e_status = " ".join(stage3e_status.split())

    stage3e_contrary_claims = deepcopy(evidence.STAGE3E_CONTRARY_CLAIMS)

    if any(
        re.search(pattern, normalized_stage3e_status, re.IGNORECASE)
        for pattern in stage3e_contrary_claims
    ):
        ctx.fail(
            "stage3i-status",
            "the promoted Stage3I receipt must not be contradicted by an appended source-ahead, stale, uncertified, uncovered, pending-promotion, Stage3H-only, Stage3G-only, or Stage3F-only claim",
        )

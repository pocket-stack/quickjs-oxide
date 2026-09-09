"""Wire evidence checks, in the ordered boundary scan."""
from __future__ import annotations

import hashlib
import re


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    def stage3e_fnv1a64(payload: bytes) -> int:
        value = 0xCBF29CE484222325
        for byte in payload:
            value ^= byte
            value = (value * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        return value
    stage3e_fnv1a64 = stage3e_fnv1a64

    def stage3e_byte_array(
        source: str,
        name: str,
        diagnostic: str = "stage3e-runtime-evidence",
    ) -> bytes:
        matches = list(re.finditer(
            rf"\bconst[ \t\n]+{re.escape(name)}[ \t\n]*:"
            r"[ \t\n]*&[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*="
            r"[ \t\n]*&[ \t\n]*\[(?P<body>[^]]*)\][ \t\n]*;",
            source,
        ))
        if len(matches) != 1:
            ctx.fail(
                diagnostic,
                f"the Rust runtime evidence must retain exactly one literal {name} wire",
            )
            return b""
        body = matches[0].group("body")
        tokens = re.findall(r"0x[0-9A-Fa-f]+|[0-9]+", body)
        residue = re.sub(r"0x[0-9A-Fa-f]+|[0-9]+|[\s,]", "", body)
        if residue:
            ctx.fail(
                diagnostic,
                f"{name} must contain only literal byte tokens",
            )
            return b""
        try:
            return bytes(int(token, 0) for token in tokens)
        except ValueError:
            ctx.fail(diagnostic, f"{name} contains an invalid byte")
            return b""
    stage3e_byte_array = stage3e_byte_array

    def stage3e_concat_hex(source: str, name: str) -> bytes:
        matches = list(re.finditer(
            rf"\bconst[ \t\n]+{re.escape(name)}[ \t\n]*:[^=;]+="
            r"[ \t\n]*concat![ \t\n]*\((?P<body>.*?)\)[ \t\n]*;",
            source,
            re.DOTALL,
        ))
        if len(matches) != 1:
            ctx.fail(
                "stage3e-runtime-evidence",
                f"the archive evidence must retain exactly one {name} concat wire",
            )
            return b""
        body = matches[0].group("body")
        chunks = re.findall(r'"([0-9A-Fa-f]*)"', body)
        residue = re.sub(r'"[0-9A-Fa-f]*"|[\s,]', "", body)
        joined = "".join(chunks)
        if residue or len(joined) % 2:
            ctx.fail(
                "stage3e-runtime-evidence",
                f"{name} must contain only an even-length literal hex concat",
            )
            return b""
        try:
            return bytes.fromhex(joined)
        except ValueError:
            ctx.fail("stage3e-runtime-evidence", f"{name} contains invalid hex")
            return b""
    stage3e_concat_hex = stage3e_concat_hex

    stage3e_runtime_source = ctx.read_source("src/engine/heap/runtime/tests.rs")

    stage3e_ordinary_source = ctx.read_source(
        "src/engine/code/binary_object/ordinary_leaf.rs"
    )

    stage3e_manual_wire = stage3e_byte_array(
        stage3e_runtime_source, "QUICKJS_ORDINARY_READ_ONLY_BC5"
    )

    stage3e_natural_wire = stage3e_byte_array(
        stage3e_runtime_source, "QUICKJS_NATURAL_READ_ONLY_BC5"
    )

    stage3e_manual_hex_wire = stage3e_concat_hex(
        stage3e_ordinary_source, "READ_ONLY_LEAF_HEX"
    )

    stage3e_natural_hex_wire = stage3e_concat_hex(
        stage3e_ordinary_source, "NATURAL_READ_ONLY_LEAF_HEX"
    )

    stage3e_wire_contracts = (
        (
            "manual raw49",
            stage3e_manual_wire,
            stage3e_manual_hex_wire,
            47,
            0xB4C1126C283093AF,
            "d05cabd4c18598b024f66eab8fd723c412fc5a469325b26fca5042507dea3ee8",
            bytes.fromhex("31f300000000"),
        ),
        (
            "natural raw49 origin",
            stage3e_natural_wire,
            stage3e_natural_hex_wire,
            58,
            0x026914EDA60A481F,
            "a07b3f39a5e3929af4899a07686e91324e4ee9c54b729f518813eaa4a1875199",
            bytes.fromhex("5e0000b3c7b41131f300000000"),
        ),
    )

    for ctx.label, runtime_wire, archive_wire, size, ctx.fnv, sha256, child_code in stage3e_wire_contracts:
        if (
            runtime_wire != archive_wire
            or len(runtime_wire) != size
            or stage3e_fnv1a64(runtime_wire) != ctx.fnv
            or hashlib.sha256(runtime_wire).hexdigest() != sha256
            or not runtime_wire.endswith(child_code)
        ):
            ctx.fail(
                "stage3e-runtime-evidence",
                f"the Rust {ctx.label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, and child code",
            )

    stage3f_nop_wire = stage3e_byte_array(
        stage3e_runtime_source, "QUICKJS_ORDINARY_NOP_BC5"
    )

    if (
        len(stage3f_nop_wire) != 41
        or stage3e_fnv1a64(stage3f_nop_wire) != 0x1C522736E3CBEF92
        or hashlib.sha256(stage3f_nop_wire).hexdigest()
        != "26c2e58ec14861dc797a7c3a3701f258ba392b649a15554256b61d7634fccdd0"
        or stage3f_nop_wire[1] != 0
        or stage3f_nop_wire[25:29] != bytes.fromhex("0c430201")
        or stage3f_nop_wire[30:37] != bytes(7)
        or stage3f_nop_wire[37:]
        != bytes.fromhex("0200b129")
    ):
        ctx.fail(
            "stage3f-runtime-evidence",
            "the Rust raw177 fixture must retain the exact 41-byte property-free zero-stack raw177/raw41 wire with no atoms or constants",
        )

    stage3g_object_wire = stage3e_byte_array(
        stage3e_runtime_source,
        "QUICKJS_ORDINARY_OBJECT_BC5",
        "stage3g-runtime-evidence",
    )

    if (
        len(stage3g_object_wire) != 41
        or stage3e_fnv1a64(stage3g_object_wire) != 0x3C41AF3FEF8B3A1E
        or hashlib.sha256(stage3g_object_wire).hexdigest()
        != "a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9"
        or not stage3g_object_wire.endswith(bytes.fromhex("0b28"))
    ):
        ctx.fail(
            "stage3g-runtime-evidence",
            "the Rust raw11 fixture must retain the exact compiler-natural 41-byte Object/Return wire with FNV-1a-64 3c41af3fef8b3a1e, SHA-256 a58ccbed5658ba6a9de99e909d5ba0b4af59ad47fccf0f5cccdff072d6494db9, and child code 0b28",
        )

    stage3h_natural_to_object_wire = stage3e_byte_array(
        stage3e_runtime_source,
        "QUICKJS_NATURAL_TO_OBJECT_BC5",
        "stage3h-runtime-evidence",
    )

    stage3h_manual_to_object_wire = stage3e_byte_array(
        stage3e_runtime_source,
        "QUICKJS_ORDINARY_TO_OBJECT_BC5",
        "stage3h-runtime-evidence",
    )

    stage3h_to_object_wire_contracts = (
        (
            "compiler-natural raw111 provenance",
            stage3h_natural_to_object_wire,
            56,
            0x65A8B3D0D7ED115A,
            "f5bdac14901bb6b752e2ca10a01dd31d6990456c43f78d5923b1da4a0ef3706e",
            bytes.fromhex("0c43020100010001020000000d0100010000"),
            bytes.fromhex("ea06116f0eea04cfeaf90ecf28"),
        ),
        (
            "mechanically derived property-free raw111",
            stage3h_manual_to_object_wire,
            46,
            0xC84F87720CD09B16,
            "13f81e66520578393a57f3290636d4778c5cae8d014591e5daaaacdd3ffd5c95",
            bytes.fromhex("0c4302010001000101000000030100010000"),
            bytes.fromhex("cf6f28"),
        ),
    )

    for ctx.label, wire, size, ctx.fnv, sha256, metadata, child_code in stage3h_to_object_wire_contracts:
        if (
            len(wire) != size
            or stage3e_fnv1a64(wire) != ctx.fnv
            or hashlib.sha256(wire).hexdigest() != sha256
            or wire[25:43] != metadata
            or not wire.endswith(child_code)
        ):
            ctx.fail(
                "stage3h-runtime-evidence",
                f"the Rust {ctx.label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, flags/frame metadata, code offset, and child body",
            )

    stage3i_push_this_wire_contracts = (
        (
            "compiler-natural strict raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_NATURAL_STRICT_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            47,
            0x4EC7E0187375D810,
            "786376192d5bfe7eb07115f62788707619ee54e8721acfa66dae1d110a580e39",
            bytes.fromhex("0c4302010000010001000000040100010000"),
            bytes.fromhex("08c7c328"),
        ),
        (
            "compiler-natural sloppy raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_NATURAL_SLOPPY_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            47,
            0x4E7F8F98ADFF8463,
            "f0430a7c241caaf94703bd5de73289d4f90fea3ee9cfaf22a660ed80df3de0a6",
            bytes.fromhex("0c4302000000010001000000040100010000"),
            bytes.fromhex("08c7c328"),
        ),
        (
            "property-free strict raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_ORDINARY_STRICT_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            41,
            0x3C3E393FEF883BC5,
            "9b14c5245a78e0a069967089cf6f89aefac3e12749d16eba36e4c15b72a3c99e",
            bytes.fromhex("0c43020100000000010000000200"),
            bytes.fromhex("0828"),
        ),
        (
            "property-free sloppy raw8",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_ORDINARY_SLOPPY_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            41,
            0x0E2485C97EEA9CFA,
            "213b3b6a332d4cf69e4c726b372c1f0087e70fc9c263a6a2193ce4763fb62648",
            bytes.fromhex("0c43020000000000010000000200"),
            bytes.fromhex("0828"),
        ),
        (
            "duplicate raw8 mismatch",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_DUPLICATE_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            43,
            0x920DE09AAF63833E,
            "9f0541bfd8a599e5f2575936d24df9a2487a1e8952fca1648afeef5c9f798a30",
            bytes.fromhex("0c43020000000000020000000400"),
            bytes.fromhex("0808a928"),
        ),
        (
            "raw8 re-entry mismatch",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_REENTER_PUSH_THIS_BC5", "stage3i-runtime-evidence"),
            65,
            0xFA100FF2B0854673,
            "32b4c9e45f5191d21aa44d3437c54b00cfa1ff4b2530d1e4cdf942a87e8f3fb4",
            bytes.fromhex("0c430200000200020200000012020001000000010000"),
            bytes.fromhex("08cf690c0000000ad3d46af5ffffffd0a928"),
        ),
    )

    for ctx.label, wire, size, ctx.fnv, sha256, metadata, child_code in stage3i_push_this_wire_contracts:
        if (
            len(wire) != size
            or stage3e_fnv1a64(wire) != ctx.fnv
            or hashlib.sha256(wire).hexdigest() != sha256
            or wire[25:25 + len(metadata)] != metadata
            or not wire.endswith(child_code)
        ):
            ctx.fail(
                "stage3i-runtime-evidence",
                f"the Rust {ctx.label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, flags/frame metadata, code offset, and child body",
            )

    stage3j_to_propkey_wire_contracts = (
        (
            "compiler-natural strict raw112",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_NATURAL_STRICT_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            50,
            0x83C33A69F73E737C,
            "7bfb0fefdbd3ff894bdcc0996707fda98153aaaeccbe50f6ade1ffaab7f818f0",
            bytes.fromhex("0c4302010001000103000000070100010000"),
            bytes.fromhex("0bcf70b44e0e28"),
        ),
        (
            "compiler-natural sloppy raw112",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_NATURAL_SLOPPY_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            50,
            0x6A1706C9AE126361,
            "c5f7a85af861402d57a8267f9af1be2d310b143a6972e5fe5d2068384b9f8fe0",
            bytes.fromhex("0c4302000001000103000000070100010000"),
            bytes.fromhex("0bcf70b44e0e28"),
        ),
        (
            "property-free strict raw112",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_ORDINARY_STRICT_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            46,
            0xC7ED09720C7CFAA1,
            "7be331650765c34157ea3731e6f86d082451e0d60e7aeb7ecd09abfe0d524cb4",
            bytes.fromhex("0c4302010001000101000000030100010000"),
            bytes.fromhex("cf7028"),
        ),
        (
            "property-free sloppy raw112",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_ORDINARY_SLOPPY_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            46,
            0xDD8BBD333D595B1C,
            "629fa63ab5c4bd4258a44e02e4171a82c7cb23ca3bce1ce11d4228e4ee10d822",
            bytes.fromhex("0c4302000001000101000000030100010000"),
            bytes.fromhex("cf7028"),
        ),
        (
            "duplicate raw112",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_DUPLICATE_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            47,
            0xC3D5F4815E807DFC,
            "b64eab0222e609fc0f5c70a2183c7558b2eecc9852d5bb795c8933e90a351ff5",
            bytes.fromhex("0c4302010001000101000000040100010000"),
            bytes.fromhex("cf707028"),
        ),
        (
            "finite-loop raw112 re-entry",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_REENTER_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            65,
            0xEDCC1B5D91F5E46D,
            "85274c3f09639ee7538bdfafd43f8bb35fc8819f9f2d4c8051e5fb140bccb638",
            bytes.fromhex("0c430201000200020200000012020001000000010000"),
            bytes.fromhex("cf70d0680d0000000e09d4cf6af4ffffff28"),
        ),
        (
            "raw112 underflow negative",
            stage3e_byte_array(stage3e_runtime_source, "QUICKJS_UNDERFLOW_TO_PROPKEY_BC5", "stage3j-runtime-evidence"),
            45,
            0x72E49B7E05FEB73D,
            "b96daff364d2ca615035e2910533e5e77b3284c52309c0d30e333275682bc841",
            bytes.fromhex("0c4302010001000101000000020100010000"),
            bytes.fromhex("7028"),
        ),
    )

    for ctx.label, wire, size, ctx.fnv, sha256, metadata, child_code in stage3j_to_propkey_wire_contracts:
        if (
            len(wire) != size
            or stage3e_fnv1a64(wire) != ctx.fnv
            or hashlib.sha256(wire).hexdigest() != sha256
            or wire[25:25 + len(metadata)] != metadata
            or not wire.endswith(child_code)
        ):
            ctx.fail(
                "stage3j-runtime-evidence",
                f"the Rust {ctx.label} fixture must retain its exact byte identity, FNV-1a-64, SHA-256, flags/frame metadata, code offset, and child body",
            )

    if (
        stage3j_to_propkey_wire_contracts[0][1][28] != 1
        or stage3j_to_propkey_wire_contracts[1][1][28] != 0
        or stage3j_to_propkey_wire_contracts[0][1][:28]
        != stage3j_to_propkey_wire_contracts[1][1][:28]
        or stage3j_to_propkey_wire_contracts[0][1][29:]
        != stage3j_to_propkey_wire_contracts[1][1][29:]
        or stage3j_to_propkey_wire_contracts[2][1][:28]
        != stage3j_to_propkey_wire_contracts[3][1][:28]
        or stage3j_to_propkey_wire_contracts[2][1][29:]
        != stage3j_to_propkey_wire_contracts[3][1][29:]
    ):
        ctx.fail(
            "stage3j-runtime-evidence",
            "the strict/sloppy natural and property-free raw112 pairs must differ only in the exact js_mode byte",
        )

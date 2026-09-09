"""Ordered rule pipeline. Later rules consume earlier authenticated observations."""
from .context import ScanContext
from .rules import source_setup, surface, native_plan, translation, ordinary_leaf, scalar, publication, runtime_protocols, runtime_contracts, wire_evidence, exception_evidence, coercion, coercion_evidence, reference_oracle, receipts, source_ownership, shared_transport

RULES = (
    ("source_setup", source_setup.check),
    ("surface", surface.check),
    ("native_plan", native_plan.check),
    ("translation", translation.check),
    ("ordinary_leaf", ordinary_leaf.check),
    ("scalar", scalar.check),
    ("publication", publication.check),
    ("runtime_protocols", runtime_protocols.check),
    ("runtime_contracts", runtime_contracts.check),
    ("wire_evidence", wire_evidence.check),
    ("exception_evidence", exception_evidence.check),
    ("coercion", coercion.check),
    ("coercion_evidence", coercion_evidence.check),
    ("reference_oracle", reference_oracle.check),
    ("receipts", receipts.check),
    ("source_ownership", source_ownership.check),
    ("shared_transport", shared_transport.check),
)


def scan(root, self_test_token=""):
    context = ScanContext(root, self_test_token)
    for name, check in RULES:
        check(context)
    return context.errors

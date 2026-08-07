#!/usr/bin/env python3
"""Dependency-free consumer for capability-negotiation-v1.

JSON Schema owns document shape. This example independently implements the semantic decision rule
that cannot be expressed as a simple field type: ordered preferences, derived eligibility, and
selection of the first eligible candidate.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

NEGOTIATION_SCHEMA = "urn:terminal-interop:capability-negotiation:v1"
RECEIPT_SCHEMA = "urn:terminal-interop:probe-receipt:v1"


class ContractError(ValueError):
    """A semantic v1 contract violation."""


def object_field(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ContractError(f"{field} must be an object")
    return value


def receipt_is_eligible(receipt: Any) -> bool:
    receipt = object_field(receipt, "receipt")
    if receipt.get("schema") != RECEIPT_SCHEMA:
        raise ContractError("candidate receipt has an unsupported schema")
    assessment = object_field(receipt.get("assessment"), "receipt.assessment")
    context = object_field(receipt.get("context"), "receipt.context")
    transport = object_field(context.get("transport"), "receipt.context.transport")
    return (
        assessment.get("availability") == "available"
        and assessment.get("conformance") == "conformant"
        and transport.get("readiness") in {"ready", "not_required"}
    )


def validate_negotiation(document: Any) -> dict[str, Any]:
    document = object_field(document, "document")
    if document.get("schema") != NEGOTIATION_SCHEMA:
        raise ContractError("unsupported negotiation schema")
    candidates = document.get("candidates")
    if not isinstance(candidates, list):
        raise ContractError("candidates must be an array")

    first_eligible: dict[str, Any] | None = None
    for preference, raw_candidate in enumerate(candidates):
        candidate = object_field(raw_candidate, f"candidates[{preference}]")
        if type(candidate.get("preference")) is not int or candidate["preference"] != preference:
            raise ContractError("candidate preferences must equal their array positions")
        eligible = receipt_is_eligible(candidate.get("receipt"))
        expected = "eligible" if eligible else "ineligible"
        if candidate.get("disposition") != expected:
            raise ContractError("candidate disposition contradicts its receipt")
        if eligible and first_eligible is None:
            first_eligible = candidate

    selection = object_field(document.get("selection"), "selection")
    if first_eligible is None:
        if selection != {"state": "no_eligible_candidate"}:
            raise ContractError("selection exists without an eligible candidate")
        return selection

    if selection.get("state") != "selected":
        raise ContractError("the first eligible candidate was not selected")
    receipt = object_field(first_eligible.get("receipt"), "selected receipt")
    if selection.get("preference") != first_eligible.get("preference"):
        raise ContractError("selection does not name the first eligible preference")
    if selection.get("capability") != receipt.get("capability"):
        raise ContractError("selected capability identity contradicts its receipt")
    if selection.get("adapter") != receipt.get("adapter"):
        raise ContractError("selected adapter identity contradicts its receipt")
    return selection


def load_document(argument: str) -> Any:
    if argument == "-":
        return json.load(sys.stdin)
    with Path(argument).open("r", encoding="utf-8") as source:
        return json.load(source)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {Path(sys.argv[0]).name} FILE|-", file=sys.stderr)
        return 2
    try:
        selection = validate_negotiation(load_document(sys.argv[1]))
    except (ContractError, json.JSONDecodeError, OSError) as error:
        print(f"invalid negotiation: {error}", file=sys.stderr)
        return 1
    print(json.dumps(selection, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

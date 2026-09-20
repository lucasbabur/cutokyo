#!/usr/bin/env python3
"""Cursor-idempotent Cutokyo source plugin example."""

from __future__ import annotations

from datetime import datetime
import json
import re
import sys
from typing import Any, NoReturn

MAX_LINE_BYTES = 1024 * 1024
MAX_ITEMS = 1000
CAPABILITIES = ["emit_observation"]
CAPABILITY_VALUES = {
    "transcript_read",
    "network",
    "emit_observation",
    "emit_derived_fact",
}
IDENTIFIER = re.compile(r"^[A-Za-z0-9_.:/@-]+$")
REQUEST_ID = re.compile(r"^[A-Za-z0-9_.:@/-]+$")


def fail() -> NoReturn:
    """Exit without copying protocol content to stderr."""
    raise SystemExit(2)


def exact_keys(
    value: dict[str, Any], allowed: set[str], required: set[str]
) -> None:
    keys = set(value)
    if len(keys) > 64 or not required <= keys or not keys <= allowed:
        fail()


def bounded_string(value: Any, maximum: int) -> str:
    if not isinstance(value, str) or not 1 <= len(value.encode()) <= maximum:
        fail()
    return value


def identifier(value: Any) -> str:
    text = bounded_string(value, 256)
    if IDENTIFIER.fullmatch(text) is None:
        fail()
    return text


def request_id(value: Any) -> str:
    text = bounded_string(value, 128)
    if REQUEST_ID.fullmatch(text) is None:
        fail()
    return text


def validate_object_bounds(value: Any) -> None:
    if isinstance(value, dict):
        if len(value) > 64:
            fail()
        for nested in value.values():
            validate_object_bounds(nested)
    elif isinstance(value, list):
        for nested in value:
            validate_object_bounds(nested)


def capabilities(value: Any) -> list[str]:
    if not isinstance(value, list) or len(value) > 8:
        fail()
    result: list[str] = []
    for capability in value:
        if not isinstance(capability, str) or capability not in CAPABILITY_VALUES:
            fail()
        if capability in result:
            fail()
        result.append(capability)
    return result


def validate_datetime(value: Any) -> None:
    text = bounded_string(value, 64)
    try:
        datetime.fromisoformat(text.replace("Z", "+00:00"))
    except ValueError:
        fail()


def validate_handshake(payload: dict[str, Any]) -> None:
    exact_keys(
        payload,
        {"plugin_id", "plugin_kind", "capabilities", "cursor_idempotent"},
        {"plugin_id", "plugin_kind", "capabilities", "cursor_idempotent"},
    )
    if identifier(payload["plugin_id"]) != "example:py-source":
        fail()
    if payload["plugin_kind"] != "source" or payload["cursor_idempotent"] is not True:
        fail()
    if capabilities(payload["capabilities"]) != CAPABILITIES:
        fail()


def validate_source_request(payload: dict[str, Any]) -> None:
    exact_keys(
        payload,
        {"operation", "harness", "cursor", "window", "granted_capabilities"},
        {"operation", "harness", "cursor", "window", "granted_capabilities"},
    )
    if payload["operation"] != "source_poll":
        fail()
    if payload["harness"] not in {"claude_code", "codex", "opencode"}:
        fail()
    cursor = payload["cursor"]
    if cursor is not None and (
        not isinstance(cursor, str) or len(cursor.encode()) > 4096
    ):
        fail()
    window = payload["window"]
    if not isinstance(window, dict):
        fail()
    exact_keys(window, {"start", "end"}, {"start", "end"})
    validate_datetime(window["start"])
    validate_datetime(window["end"])
    granted = capabilities(payload["granted_capabilities"])
    if "emit_observation" not in granted:
        fail()


def validate_telemetry(value: Any) -> None:
    if not isinstance(value, dict):
        fail()
    exact_keys(value, {"name", "value", "unit"}, {"name", "value"})
    name = bounded_string(value["name"], 128)
    if re.fullmatch(r"[A-Za-z0-9_.:-]+", name) is None:
        fail()
    number = value["value"]
    if isinstance(number, bool) or not isinstance(number, int) or not 0 <= number <= 9007199254740991:
        fail()
    if "unit" in value:
        bounded_string(value["unit"], 32)


def validate_response(payload: dict[str, Any]) -> None:
    exact_keys(
        payload,
        {
            "status",
            "cursor",
            "observations",
            "tags",
            "derived_facts",
            "used_capabilities",
            "telemetry",
        },
        {"status", "used_capabilities"},
    )
    if payload["status"] not in {"completed", "cancelled"}:
        fail()
    used = capabilities(payload["used_capabilities"])
    if any(capability != "emit_observation" for capability in used):
        fail()
    cursor = payload.get("cursor")
    if cursor is not None and (
        not isinstance(cursor, str) or len(cursor.encode()) > 4096
    ):
        fail()
    if "tags" in payload or "derived_facts" in payload:
        fail()
    observations = payload.get("observations", [])
    if not isinstance(observations, list) or len(observations) > MAX_ITEMS:
        fail()
    for observation in observations:
        if not isinstance(observation, dict):
            fail()
        exact_keys(
            observation,
            {"observation_id", "kind", "payload"},
            {"observation_id", "kind", "payload"},
        )
        identifier(observation["observation_id"])
        bounded_string(observation["kind"], 128)
        if not isinstance(observation["payload"], dict):
            fail()
        validate_object_bounds(observation["payload"])
    if observations and "emit_observation" not in used:
        fail()
    if "telemetry" in payload:
        validate_telemetry(payload["telemetry"])


def validate_message(value: Any, direction: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail()
    exact_keys(
        value,
        {"protocol_major", "kind", "request_id", "payload"},
        {"protocol_major", "kind", "request_id", "payload"},
    )
    if value["protocol_major"] != 1:
        fail()
    request_id(value["request_id"])
    payload = value["payload"]
    if not isinstance(payload, dict):
        fail()
    validate_object_bounds(payload)
    kind = value["kind"]
    if direction == "incoming":
        if kind == "request":
            validate_source_request(payload)
        elif kind == "cancel":
            exact_keys(payload, set(), set())
        else:
            fail()
    elif direction == "outgoing":
        if kind == "handshake":
            validate_handshake(payload)
        elif kind == "response":
            validate_response(payload)
        else:
            fail()
    else:
        fail()
    return value


def send(value: dict[str, Any]) -> None:
    validate_message(value, "outgoing")
    encoded = json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()
    if len(encoded) + 1 > MAX_LINE_BYTES:
        fail()
    sys.stdout.buffer.write(encoded + b"\n")
    sys.stdout.buffer.flush()


send(
    {
        "protocol_major": 1,
        "kind": "handshake",
        "request_id": "handshake",
        "payload": {
            "plugin_id": "example:py-source",
            "plugin_kind": "source",
            "capabilities": CAPABILITIES,
            "cursor_idempotent": True,
        },
    }
)

for raw_line in sys.stdin.buffer:
    if len(raw_line) > MAX_LINE_BYTES or not raw_line.endswith(b"\n"):
        fail()
    try:
        incoming = validate_message(json.loads(raw_line), "incoming")
    except (UnicodeDecodeError, json.JSONDecodeError):
        fail()
    if incoming["kind"] == "cancel":
        send(
            {
                "protocol_major": 1,
                "kind": "response",
                "request_id": incoming["request_id"],
                "payload": {"status": "cancelled", "used_capabilities": []},
            }
        )
        continue
    payload = incoming["payload"]
    cursor = payload["cursor"]
    window = payload["window"]
    if cursor != "verify-cursor":
        fail()
    # The stable ID and payload depend only on cursor/window, never request_id.
    send(
        {
            "protocol_major": 1,
            "kind": "response",
            "request_id": incoming["request_id"],
            "payload": {
                "status": "completed",
                "cursor": "verify-cursor:next",
                "used_capabilities": ["emit_observation"],
                "observations": [
                    {
                        "observation_id": "obs:example:py-source:verify-cursor",
                        "kind": "example.source_event",
                        "payload": {
                            "cursor": cursor,
                            "window_start": window["start"],
                            "window_end": window["end"],
                        },
                    }
                ],
            },
        }
    )

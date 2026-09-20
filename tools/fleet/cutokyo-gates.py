#!/usr/bin/env python3
"""Static acceptance gates for the Cutokyo repository contract.

Exit 0 means every selected gate passed; any nonzero exit means the repository
violates the contract or the requested input could not be evaluated. Behavioral,
UI, packaging, and harness checks remain first-class project tests; this tool
protects the architectural and open-source skeleton they rely on.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import tomllib

REQUIRED_PATHS = (
    "Cargo.toml",
    "rust-toolchain.toml",
    "deny.toml",
    "lefthook.yml",
    "LICENSE",
    "NOTICE",
    "TRADEMARK.md",
    "THIRD_PARTY.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "README.md",
    "crates/cutokyo-domain/Cargo.toml",
    "crates/cutokyo-domain/src/lib.rs",
    "crates/cutokyo-core/Cargo.toml",
    "crates/cutokyo-cli/Cargo.toml",
    "crates/cutokyo-desktop/Cargo.toml",
    "ui/package.json",
    "schemas/plugin-protocol.v1.json",
    "schemas/spool-line.v1.json",
    "schemas/domain/records.v1.json",
    "schemas/settings/settings-patch.v1.json",
    "fixtures/registry.json",
    "fixtures/harness",
    "fixtures/spool",
    "fixtures/db",
    "tools/fake-harness/Cargo.toml",
    "examples/plugins/ts-processor",
    "examples/plugins/py-source",
    "tests/Cargo.toml",
    "tests/src/lib.rs",
    "tests/pipeline.rs",
    "tests/e2e.rs",
    "tests/upgrade.rs",
    "tests/architecture.rs",
    "docs/spec/product.md",
    "docs/plugin-authoring.md",
    "docs/provenance/clean-room-policy.md",
    "docs/provenance/reference-observations.md",
    "docs/provenance/decision-ledger.md",
    "docs/adr/0001-local-first-single-writer.md",
    "docs/adr/0002-native-capture-before-proxy.md",
    "docs/adr/0003-versioned-raw-observations.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    ".github/ISSUE_TEMPLATE/bug_report.yml",
    ".github/ISSUE_TEMPLATE/feature_request.yml",
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
    ".github/workflows/drift.yml",
    ".github/workflows/dco.yml",
)

WORKSPACE_MEMBERS = {
    "crates/cutokyo-domain",
    "crates/cutokyo-core",
    "crates/cutokyo-cli",
    "crates/cutokyo-desktop",
    "tools/fake-harness",
    "tests",
}

FORBIDDEN_DOMAIN_DEPENDENCIES = {
    "rusqlite",
    "rusqlite_migration",
    "sqlx",
    "reqwest",
    "hyper",
    "tokio",
    "tauri",
    "ureq",
}

FORBIDDEN_DOMAIN_SOURCE = {
    "std::fs": "filesystem",
    "std::net": "network",
    "std::process": "process execution",
    "rusqlite": "SQLite",
    "sqlx": "SQL",
    "reqwest": "HTTP",
    "tokio::fs": "async filesystem",
    "tokio::net": "async network",
    "tauri::": "desktop shell",
}

SCHEMA_CONTRACTS = {
    "schemas/spool-line.v1.json": {
        "format_version",
        "observation_id",
        "harness",
        "session_key",
        "observed_at",
        "kind",
        "payload",
    },
    "schemas/plugin-protocol.v1.json": {
        "protocol_major",
        "kind",
        "request_id",
        "payload",
    },
    "schemas/domain/records.v1.json": {
        "record_version",
        "record_type",
        "record",
    },
    "schemas/settings/settings-patch.v1.json": set(),
    "schemas/cli-output.v1.json": {"schema_version", "command", "ok", "meta"},
}

CI_TOKENS = {
    ".github/workflows/ci.yml": (
        "ubuntu-latest",
        "macos-latest",
        "windows-latest",
        "cargo fmt --check",
        "cargo clippy",
        "cargo test",
        "cargo deny check licenses bans",
        "tsc --noEmit",
        "oxlint",
        "prettier",
        "vitest",
        "typos",
    ),
    ".github/workflows/release.yml": (
        "cargo-dist",
        "tauri-apps/tauri-action@",
        "attest-build-provenance",
        "sha256",
        "sbom",
    ),
    ".github/workflows/drift.yml": (
        "schedule:",
        "issues: write",
        "claude",
        "codex",
        "opencode",
    ),
    ".github/workflows/dco.yml": (
        "pull_request",
        "actions-dco",
    ),
}

PRODUCT_TOKENS = (
    "local-only",
    "claude code",
    "codex",
    "opencode",
    "session history",
    "resume",
    "proxy",
    "explicit consent",
    "community",
)


@dataclass(frozen=True)
class GateResult:
    name: str
    errors: tuple[str, ...]

    @property
    def passed(self) -> bool:
        return not self.errors


def _read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as exc:
        raise ValueError(f"cannot read {path}: {exc}") from exc


def gate_layout(root: Path) -> GateResult:
    errors = [f"missing required path: {relative}" for relative in REQUIRED_PATHS if not (root / relative).exists()]

    cargo_path = root / "Cargo.toml"
    if cargo_path.is_file():
        try:
            cargo = tomllib.loads(_read_text(cargo_path))
            members = set(cargo.get("workspace", {}).get("members", []))
            missing = sorted(WORKSPACE_MEMBERS - members)
            if missing:
                errors.append(f"workspace missing members: {', '.join(missing)}")
        except (tomllib.TOMLDecodeError, ValueError) as exc:
            errors.append(f"invalid root Cargo.toml: {exc}")

    tests_manifest = root / "tests/Cargo.toml"
    if tests_manifest.is_file():
        try:
            tests_cargo = tomllib.loads(_read_text(tests_manifest))
            if tests_cargo.get("package", {}).get("name") != "cutokyo-integration-tests":
                errors.append("tests/Cargo.toml package must be named cutokyo-integration-tests")
            declared_tests = {
                target.get("name"): target.get("path")
                for target in tests_cargo.get("test", [])
                if isinstance(target, dict)
            }
            expected_tests = {
                "pipeline": "pipeline.rs",
                "e2e": "e2e.rs",
                "upgrade": "upgrade.rs",
                "architecture": "architecture.rs",
            }
            if declared_tests != expected_tests:
                errors.append("tests/Cargo.toml must declare the four root-level integration test targets")
        except (tomllib.TOMLDecodeError, ValueError) as exc:
            errors.append(f"invalid tests/Cargo.toml: {exc}")

    return GateResult("layout", tuple(errors))


def gate_architecture(root: Path) -> GateResult:
    errors: list[str] = []
    cargo_path = root / "crates/cutokyo-domain/Cargo.toml"
    if not cargo_path.is_file():
        errors.append("missing cutokyo-domain Cargo.toml")
    else:
        try:
            cargo = tomllib.loads(_read_text(cargo_path))
            dependency_sections = (
                cargo.get("dependencies", {}),
                cargo.get("build-dependencies", {}),
                cargo.get("target", {}),
            )
            serialized = json.dumps(dependency_sections).lower().replace("-", "_")
            for dependency in sorted(FORBIDDEN_DOMAIN_DEPENDENCIES):
                if f'"{dependency}"' in serialized:
                    errors.append(f"cutokyo-domain depends on forbidden I/O crate: {dependency}")
        except (tomllib.TOMLDecodeError, ValueError) as exc:
            errors.append(f"invalid cutokyo-domain Cargo.toml: {exc}")

    source_dir = root / "crates/cutokyo-domain/src"
    if not source_dir.is_dir():
        errors.append("missing cutokyo-domain source directory")
    else:
        for path in sorted(source_dir.rglob("*.rs")):
            try:
                text = _read_text(path)
            except ValueError as exc:
                errors.append(str(exc))
                continue
            for token, capability in FORBIDDEN_DOMAIN_SOURCE.items():
                if token in text:
                    relative = path.relative_to(root)
                    errors.append(f"{relative} imports {capability} through forbidden token {token!r}")

    return GateResult("architecture", tuple(errors))


def gate_contracts(root: Path) -> GateResult:
    errors: list[str] = []
    schema_ids: dict[str, str] = {}
    for relative, expected_required in SCHEMA_CONTRACTS.items():
        path = root / relative
        if not path.is_file():
            errors.append(f"missing schema: {relative}")
            continue
        try:
            schema = json.loads(_read_text(path))
        except (json.JSONDecodeError, ValueError) as exc:
            errors.append(f"invalid JSON schema {relative}: {exc}")
            continue
        if not isinstance(schema, dict):
            errors.append(f"{relative} root must be a JSON object")
            continue
        if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
            errors.append(f"{relative} must declare JSON Schema 2020-12")
        schema_id = schema.get("$id")
        if not isinstance(schema_id, str) or not schema_id:
            errors.append(f"{relative} must declare a stable $id")
        elif schema_id in schema_ids:
            errors.append(f"{relative} duplicates $id from {schema_ids[schema_id]}")
        else:
            schema_ids[schema_id] = relative
        if schema.get("type") != "object":
            errors.append(f"{relative} root type must be object")
        required_value = schema.get("required", [])
        properties_value = schema.get("properties", {})
        if not isinstance(required_value, list) or not all(isinstance(item, str) for item in required_value):
            errors.append(f"{relative} required must be an array of field names")
            continue
        if not isinstance(properties_value, dict):
            errors.append(f"{relative} properties must be an object")
            continue
        required = set(required_value)
        missing = sorted(expected_required - required)
        if missing:
            errors.append(f"{relative} missing required fields: {', '.join(missing)}")
        properties = set(properties_value)
        undeclared = sorted(required - properties)
        if undeclared:
            errors.append(f"{relative} requires undeclared properties: {', '.join(undeclared)}")

    registry_path = root / "fixtures/registry.json"
    registered_schemas: set[str] = set()
    if not registry_path.is_file():
        errors.append("missing fixture registry: fixtures/registry.json")
    else:
        try:
            registry = json.loads(_read_text(registry_path))
        except (json.JSONDecodeError, ValueError) as exc:
            errors.append(f"invalid fixture registry: {exc}")
            registry = None
        if not isinstance(registry, dict):
            if registry is not None:
                errors.append("fixture registry root must be an object")
        elif registry.get("registry_version") != 1 or not isinstance(registry.get("contracts"), list):
            errors.append("fixture registry must declare version 1 and a contracts array")
        else:
            for index, contract in enumerate(registry["contracts"]):
                label = f"fixture registry contract {index}"
                if not isinstance(contract, dict):
                    errors.append(f"{label} must be an object")
                    continue
                name = contract.get("name")
                schema = contract.get("schema")
                good = contract.get("good")
                bad = contract.get("bad")
                if not isinstance(name, str) or not name:
                    errors.append(f"{label} must have a nonempty name")
                if not isinstance(schema, str) or schema not in SCHEMA_CONTRACTS:
                    errors.append(f"{label} references an unknown schema")
                elif schema in registered_schemas:
                    errors.append(f"fixture registry repeats schema: {schema}")
                else:
                    registered_schemas.add(schema)
                for kind, fixtures in (("good", good), ("bad", bad)):
                    if not isinstance(fixtures, list) or not fixtures or not all(isinstance(item, str) for item in fixtures):
                        errors.append(f"{label} must register at least one {kind} fixture")
                        continue
                    for fixture in fixtures:
                        relative = Path(fixture)
                        if relative.is_absolute() or ".." in relative.parts:
                            errors.append(f"{label} has unsafe fixture path: {fixture}")
                            continue
                        fixture_path = root / relative
                        if not fixture_path.is_file():
                            errors.append(f"missing {kind} fixture: {fixture}")
                            continue
                        try:
                            text = _read_text(fixture_path)
                        except ValueError as exc:
                            errors.append(str(exc))
                            continue
                        if not text:
                            errors.append(f"empty {kind} fixture: {fixture}")
                        if kind == "good":
                            try:
                                json.loads(text)
                            except json.JSONDecodeError as exc:
                                errors.append(f"invalid good JSON fixture {fixture}: {exc}")
                            if schema == "schemas/spool-line.v1.json" and (not text.endswith("\n") or text.count("\n") != 1):
                                errors.append(f"spool fixture must contain exactly one JSON line: {fixture}")

    missing_registry_schemas = sorted(set(SCHEMA_CONTRACTS) - registered_schemas)
    if missing_registry_schemas:
        errors.append(f"fixture registry missing schemas: {', '.join(missing_registry_schemas)}")

    disk_schemas = {
        str(path.relative_to(root))
        for path in (root / "schemas").rglob("*.json")
    } if (root / "schemas").is_dir() else set()
    unregistered_disk_schemas = sorted(disk_schemas - set(SCHEMA_CONTRACTS))
    if unregistered_disk_schemas:
        errors.append(f"unregistered schemas: {', '.join(unregistered_disk_schemas)}")

    return GateResult("contracts", tuple(errors))


def gate_legal(root: Path) -> GateResult:
    errors: list[str] = []
    checks = {
        "LICENSE": ("Apache License", "Version 2.0, January 2004"),
        "CONTRIBUTING.md": ("Developer Certificate of Origin", "git commit -s", "Signed-off-by"),
        "NOTICE": ("Cutokyo",),
        "TRADEMARK.md": ("Cutokyo", "trademark"),
        "THIRD_PARTY.md": ("third-party", "license"),
        "docs/provenance/clean-room-policy.md": ("clean-room", "no code"),
        "docs/provenance/reference-observations.md": ("reference observation", "independently"),
        "docs/provenance/decision-ledger.md": ("implementation", "test"),
    }
    for relative, tokens in checks.items():
        path = root / relative
        if not path.is_file():
            errors.append(f"missing legal file: {relative}")
            continue
        try:
            text = _read_text(path)
        except ValueError as exc:
            errors.append(str(exc))
            continue
        lowered = text.lower()
        for token in tokens:
            if token.lower() not in lowered:
                errors.append(f"{relative} missing required text: {token}")
    return GateResult("legal", tuple(errors))


def gate_ci(root: Path) -> GateResult:
    errors: list[str] = []
    for relative, tokens in CI_TOKENS.items():
        path = root / relative
        if not path.is_file():
            errors.append(f"missing workflow: {relative}")
            continue
        try:
            lowered = _read_text(path).lower()
        except ValueError as exc:
            errors.append(str(exc))
            continue
        for token in tokens:
            if token.lower() not in lowered:
                errors.append(f"{relative} missing required capability token: {token}")
    return GateResult("ci", tuple(errors))


def gate_scope(root: Path) -> GateResult:
    errors: list[str] = []
    spec_path = root / "docs/spec/product.md"
    if not spec_path.is_file():
        return GateResult("scope", ("missing docs/spec/product.md",))
    try:
        lowered = _read_text(spec_path).lower()
    except ValueError as exc:
        return GateResult("scope", (str(exc),))
    for token in PRODUCT_TOKENS:
        if token not in lowered:
            errors.append(f"product spec missing explicit scope term: {token}")
    return GateResult("scope", tuple(errors))


GATES: dict[str, Callable[[Path], GateResult]] = {
    "layout": gate_layout,
    "architecture": gate_architecture,
    "contracts": gate_contracts,
    "legal": gate_legal,
    "ci": gate_ci,
    "scope": gate_scope,
}


def run(root: Path, names: list[str]) -> list[GateResult]:
    return [GATES[name](root) for name in names]


def _write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _schema(required: set[str], schema_id: str) -> str:
    return json.dumps(
        {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": schema_id,
            "title": schema_id,
            "type": "object",
            "properties": {name: {} for name in sorted(required)},
            "required": sorted(required),
        },
        indent=2,
    )


def _make_good_fixture(root: Path) -> None:
    directory_paths = {
        "fixtures/harness",
        "fixtures/spool",
        "fixtures/db",
        "examples/plugins/ts-processor",
        "examples/plugins/py-source",
    }
    for relative in REQUIRED_PATHS:
        path = root / relative
        if relative in directory_paths:
            path.mkdir(parents=True, exist_ok=True)
        else:
            _write(path, "fixture\n")

    members = "\n".join(f'  "{member}",' for member in sorted(WORKSPACE_MEMBERS))
    _write(root / "Cargo.toml", f"[workspace]\nmembers = [\n{members}\n]\nresolver = \"2\"\n")
    _write(root / "crates/cutokyo-domain/Cargo.toml", "[package]\nname = \"cutokyo-domain\"\nversion = \"0.1.0\"\nedition = \"2024\"\n")
    _write(root / "crates/cutokyo-domain/src/lib.rs", "pub struct SessionId(pub String);\n")
    integration_tests = """[package]
name = "cutokyo-integration-tests"
version = "0.1.0"
edition = "2024"
publish = false

[[test]]
name = "pipeline"
path = "pipeline.rs"

[[test]]
name = "e2e"
path = "e2e.rs"

[[test]]
name = "upgrade"
path = "upgrade.rs"

[[test]]
name = "architecture"
path = "architecture.rs"
"""
    _write(root / "tests/Cargo.toml", integration_tests)
    _write(root / "tests/src/lib.rs", "")

    for relative, required in SCHEMA_CONTRACTS.items():
        _write(root / relative, _schema(required, f"https://cutokyo.dev/schemas/{Path(relative).name}"))

    fixture_contracts = []
    for index, relative in enumerate(SCHEMA_CONTRACTS):
        stem = Path(relative).stem
        good = f"fixtures/contracts/{stem}/good.jsonl" if relative == "schemas/spool-line.v1.json" else f"fixtures/contracts/{stem}/good.json"
        bad = f"fixtures/contracts/{stem}/bad.json"
        _write(root / good, "{}\n")
        _write(root / bad, "{}\n")
        fixture_contracts.append({"name": f"contract-{index}", "schema": relative, "good": [good], "bad": [bad]})
    _write(
        root / "fixtures/registry.json",
        json.dumps({"registry_version": 1, "contracts": fixture_contracts}, indent=2),
    )

    _write(root / "LICENSE", "Apache License\nVersion 2.0, January 2004\n")
    _write(root / "NOTICE", "Cutokyo\nCopyright contributors\n")
    _write(root / "TRADEMARK.md", "# Cutokyo trademark policy\n")
    _write(root / "THIRD_PARTY.md", "# Third-party licenses\n")
    _write(
        root / "CONTRIBUTING.md",
        "Developer Certificate of Origin\nUse git commit -s to add Signed-off-by.\n",
    )
    _write(root / "docs/provenance/clean-room-policy.md", "# Clean-room policy\nNo code is copied from the predecessor.\n")
    _write(root / "docs/provenance/reference-observations.md", "# Reference observations\nEvery behavior is independently specified.\n")
    _write(root / "docs/provenance/decision-ledger.md", "# Decision ledger\nRecord each implementation and test.\n")
    _write(root / "docs/spec/product.md", "\n".join(PRODUCT_TOKENS))

    for relative, tokens in CI_TOKENS.items():
        _write(root / relative, "\n".join(tokens))


def selftest() -> tuple[bool, list[str]]:
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="cutokyo-gates-") as directory:
        root = Path(directory)
        _make_good_fixture(root)

        good = run(root, list(GATES))
        for result in good:
            if not result.passed:
                failures.append(f"known-good fixture failed {result.name}: {'; '.join(result.errors)}")

        mutations: list[tuple[str, Callable[[Path], None]]] = [
            ("layout", lambda fixture: shutil.rmtree(fixture / "crates/cutokyo-cli")),
            (
                "architecture",
                lambda fixture: _write(
                    fixture / "crates/cutokyo-domain/Cargo.toml",
                    "[package]\nname = \"cutokyo-domain\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nrusqlite = \"0.40\"\n",
                ),
            ),
            (
                "contracts",
                lambda fixture: _write(
                    fixture / "schemas/spool-line.v1.json",
                    _schema({"format_version"}, "https://cutokyo.dev/schemas/spool-line.v1.json"),
                ),
            ),
            (
                "contracts",
                lambda fixture: (fixture / "fixtures/contracts/spool-line.v1/good.jsonl").unlink(),
            ),
            (
                "contracts",
                lambda fixture: _write(fixture / "fixtures/registry.json", "not json\n"),
            ),
            ("legal", lambda fixture: _write(fixture / "LICENSE", "all rights reserved\n")),
            (
                "ci",
                lambda fixture: _write(
                    fixture / ".github/workflows/ci.yml",
                    _read_text(fixture / ".github/workflows/ci.yml").replace("windows-latest", "windows-omitted"),
                ),
            ),
            (
                "scope",
                lambda fixture: _write(
                    fixture / "docs/spec/product.md",
                    _read_text(fixture / "docs/spec/product.md").replace("explicit consent", "implicit activation"),
                ),
            ),
        ]

        for index, (gate_name, mutate) in enumerate(mutations):
            candidate = root / f"bad-{index}-{gate_name}"
            shutil.copytree(root, candidate, ignore=shutil.ignore_patterns("bad-*"))
            mutate(candidate)
            result = GATES[gate_name](candidate)
            if result.passed:
                failures.append(f"known-bad mutation was accepted by {gate_name}")

    return not failures, failures


def _emit(results: list[GateResult], as_json: bool) -> None:
    if as_json:
        print(
            json.dumps(
                {
                    "passed": all(result.passed for result in results),
                    "gates": [
                        {"name": result.name, "passed": result.passed, "errors": list(result.errors)}
                        for result in results
                    ],
                },
                indent=2,
            )
        )
        return

    for result in results:
        state = "PASS" if result.passed else "FAIL"
        print(f"[{state}] {result.name}")
        for error in result.errors:
            print(f"  - {error}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("gate", choices=["all", "selftest", *GATES])
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--json", action="store_true", help="emit machine-readable results")
    args = parser.parse_args(argv)

    if args.gate == "selftest":
        passed, failures = selftest()
        if args.json:
            print(json.dumps({"passed": passed, "failures": failures}, indent=2))
        else:
            print("[PASS] selftest" if passed else "[FAIL] selftest")
            for failure in failures:
                print(f"  - {failure}")
        return 0 if passed else 1

    root = Path(args.root).expanduser().resolve()
    if not root.is_dir():
        message = f"unusable repository root: {root}"
        if args.json:
            print(json.dumps({"passed": False, "error": message}, indent=2))
        else:
            print(message, file=sys.stderr)
        return 2

    names = list(GATES) if args.gate == "all" else [args.gate]
    results = run(root, names)
    _emit(results, args.json)
    return 0 if all(result.passed for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())

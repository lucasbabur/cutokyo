#!/usr/bin/env python3
"""Parse release workflow structure and exercise its platform/signing invariants.

Actionlint remains the full GitHub expression/Bash checker. --powershell adds the
real PowerShell parser on Windows; Linux success never claims that parser ran.
"""
from __future__ import annotations

import argparse
import copy
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

import yaml

PIN = re.compile(r"^[^@]+@[0-9a-f]{40}$")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def validate(ci: dict, release: dict) -> None:
    for workflow in (ci, release):
        require(isinstance(workflow.get("jobs"), dict), "workflow jobs must be a mapping")
        for job in workflow["jobs"].values():
            for step in job.get("steps", []):
                if "uses" in step:
                    require(PIN.fullmatch(step["uses"]) is not None, "actions must be pinned to reviewed full revisions")
    platforms = ci["jobs"]["build-test"]["strategy"]["matrix"]["os"]
    require(all(any(os.startswith(family) for os in platforms) for family in ("ubuntu", "macos", "windows")),
            "CI must build/test Linux, macOS and Windows")
    expected = {
        "linux-x86_64": "x86_64-unknown-linux-gnu",
        "darwin-aarch64": "aarch64-apple-darwin",
        "windows-x86_64": "x86_64-pc-windows-msvc",
    }
    tauri = release["jobs"]["tauri-artifacts"]
    matrix = tauri["strategy"]["matrix"]["include"]
    require({row["updater-platform"]: row.get("target") for row in matrix} == expected,
            "Tauri updater architecture must match explicit build targets")
    require(next(row for row in matrix if row["updater-platform"] == "darwin-aarch64")["os"] == "macos-14",
            "Darwin ARM artifacts need an ARM runner")
    builds = [step for step in tauri["steps"] if "tauri build" in step.get("run", "")]
    require(len(builds) == 3, "three dry-run/Unix-signed/Windows-signed Tauri recipes are required")
    for build in builds:
        recipe = build["run"]
        require('--target "${{ matrix.target }}"' in recipe, "Tauri recipes must build the declared target")
        require("--debug" not in recipe and "native-e2e" not in recipe,
                "release artifacts must use the production profile and frontend")
        require("source-snapshot.py" in recipe, "Tauri builds need source immutability proof")
    for workflow in (ci, release):
        for job in workflow["jobs"].values():
            # Detect Bash interpolation in default-shell cross-platform steps.
            matrix_text = json.dumps(job.get("strategy", {}))
            if "windows" not in matrix_text.lower():
                continue
            for step in job["steps"]:
                condition = step.get("if", "")
                if "shell" in step or "runner.os != 'Windows'" in condition or "runner.os == 'Linux'" in condition:
                    continue
                recipe = step.get("run", "")
                require(not re.search(r"\$(?:CARGO_DIST_VERSION|RUNNER_TEMP|CARGO_TARGET_DIR)\b", recipe),
                        "default Windows shell cannot expand Bash environment variables")
                require(not re.search(r"\\\s*\n", recipe), "default Windows shell cannot use Bash line continuation")
    final = release["jobs"]["release-manifest"]["steps"]
    publish = [step for step in final if "npm publish" in step.get("run", "")]
    require(len(publish) == 1 and publish[0].get("if") == "needs.release-contract.outputs.publish == 'true'",
            "publication must be gated on the immutable tag preflight")
    assembly = next(step["run"] for step in final if "manifest_args=(" in step.get("run", ""))
    require("'*.AppImage'" in assembly and "'*.msi'" in assembly and "'*.app.tar.gz'" in assembly,
            "Tauri v2 native updater formats must match manifest lookup")
    require("--verify-only" in assembly and "--signature-verifier" in assembly,
            "release assembly must cryptographically verify and revalidate the final inventory")
    preflight = next(step for step in release["jobs"]["release-contract"]["steps"]
                     if "required signing/publication secrets are absent" in step.get("run", ""))
    require(preflight.get("if") == "steps.release.outputs.publish == 'true'" and "exit 78" in preflight["run"],
            "missing signing material must block tag publication with exit 78")


def parse_powershell(workflows: list[dict]) -> int:
    count = 0
    with tempfile.TemporaryDirectory(prefix="cutokyo-pwsh-syntax-") as temporary:
        script = Path(temporary) / "candidate.ps1"
        for workflow in workflows:
            for job in workflow["jobs"].values():
                windows = "windows" in json.dumps(job.get("strategy", {})).lower()
                for step in job["steps"]:
                    shell = step.get("shell", "pwsh" if windows else "bash")
                    if shell != "pwsh" or "run" not in step:
                        continue
                    candidate = re.sub(r"\$\{\{.*?\}\}", "expression_value", step["run"])
                    script.write_text(candidate, encoding="utf-8")
                    command = ("$tokens=$null; $errors=$null; "
                               "[System.Management.Automation.Language.Parser]::ParseFile("
                               " $args[0], [ref]$tokens, [ref]$errors) | Out-Null; "
                               "if ($errors.Count) { $errors | Out-String | Write-Error; exit 1 }")
                    # A script file avoids cmd.exe/PowerShell argument interpolation.
                    parser = Path(temporary) / "parse.ps1"
                    parser.write_text(command, encoding="utf-8")
                    subprocess.run(["pwsh", "-NoProfile", "-NonInteractive", "-File", str(parser), str(script)], check=True)
                    count += 1
    require(count > 0, "no PowerShell workflow steps were parsed")
    return count


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--powershell", action="store_true")
    args = parser.parse_args()
    workflows = [yaml.load((args.root / f".github/workflows/{name}.yml").read_text(), Loader=yaml.BaseLoader)
                 for name in ("ci", "release")]
    ci, release = workflows
    validate(ci, release)
    # Each mutation starts from a fully validated good document.
    mutations = [
        lambda c, r: c["jobs"]["build-test"]["strategy"]["matrix"].update(os=["ubuntu-latest"]),
        lambda c, r: r["jobs"]["tauri-artifacts"]["strategy"]["matrix"]["include"][1].update(target="x86_64-apple-darwin"),
        lambda c, r: r["jobs"]["release-contract"]["steps"][0].update(uses="actions/checkout@main"),
        lambda c, r: next(s for s in r["jobs"]["release-manifest"]["steps"] if "npm publish" in s.get("run", "")).pop("if"),
    ]
    for mutation in mutations:
        bad_ci, bad_release = copy.deepcopy(workflows)
        mutation(bad_ci, bad_release)
        try:
            validate(bad_ci, bad_release)
        except ValueError:
            pass
        else:
            raise ValueError("workflow mutation escaped detection")
    count = parse_powershell(workflows) if args.powershell else 0
    print(json.dumps({"workflow_documents": 2, "rejected_mutations": len(mutations),
                      "powershell_steps_parsed": count, "remote_jobs_executed": False}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, TypeError, yaml.YAMLError) as error:
        print(f"workflow-contract: {error}", file=sys.stderr)
        raise SystemExit(1)

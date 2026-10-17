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
    builds = [step for step in tauri["steps"] if "tauri build" in step.get("run", "")
              or step.get("uses", "").startswith("tauri-apps/tauri-action@")]
    require(len(builds) == 3, "three dry-run/Unix-signed/Windows-signed Tauri recipes are required")
    for build in builds:
        options = build.get("with", {})
        recipe = build.get("run", options.get("args", ""))
        require('--target "${{ matrix.target }}"' in recipe, "Tauri recipes must build the declared target")
        require("--debug" not in recipe and "native-e2e" not in recipe,
                "release artifacts must use the production profile and frontend")
        if "uses" not in build:
            require("source-snapshot.py" in recipe, "Tauri builds need source immutability proof")
            continue
        require(options.get("tauriScript") ==
                "python tools/scripts/source-snapshot.py --root . --manifest ../tauri-dry-run-source.json -- pnpm exec tauri"
                and options.get("projectPath") == ".",
                "Tauri action must wrap the pinned workspace CLI in the continuous source guard")
        require(options.get("includeDebug") == "false" and options.get("includeRelease") == "true"
                and options.get("includeUpdaterJson") == "false" and options.get("retryAttempts") == "0",
                "Tauri action must build production bytes once without publishing updater metadata")
        require(not ({"tagName", "releaseId", "releaseName"} & options.keys())
                and build.get("if") == "needs.release-contract.outputs.publish != 'true'",
                "Tauri action is dry-run only and must not publish")
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
    publish = [step for step in final if "gh release create" in step.get("run", "")]
    require(len(publish) == 1 and publish[0].get("if") == "needs.release-contract.outputs.publish == 'true'",
            "publication must be gated on the immutable tag preflight")
    smoke = next(i for i, step in enumerate(final) if "smoke-npm.py" in step.get("run", ""))
    require(any("kernel.apparmor_restrict_unprivileged_userns=0" in step.get("run", "")
                for step in final[:smoke]), "release smoke requires the Ubuntu namespace fix")
    require("npm publish" not in json.dumps(release) and "NPM_TOKEN" not in json.dumps(release),
            "desktop signing must not gate npm publication")
    assembly = next(step["run"] for step in final if "manifest_args=(" in step.get("run", ""))
    require("'*.AppImage'" in assembly and "'*.msi'" in assembly and "'*.app.tar.gz'" in assembly,
            "Tauri v2 native updater formats must match manifest lookup")
    require("--verify-only" in assembly and "--signature-verifier" in assembly,
            "release assembly must cryptographically verify and revalidate the final inventory")
    preflight = next(step for step in release["jobs"]["release-contract"]["steps"]
                     if "required signing/publication secrets are absent" in step.get("run", ""))
    require(preflight.get("if") == "steps.release.outputs.publish == 'true'" and "exit 78" in preflight["run"],
            "missing signing material must block tag publication with exit 78")


def validate_npm(workflow: dict) -> None:
    require(set(workflow["on"]) == {"workflow_dispatch"}, "npm publication must be explicitly dispatched")
    require(workflow["on"]["workflow_dispatch"]["inputs"]["mode"]["default"] == "dry-run",
            "npm must default to dry-run")
    require("secrets." not in json.dumps(workflow), "npm must use OIDC without desktop/npm secrets")
    for job in workflow["jobs"].values():
        for step in job.get("steps", []):
            if "uses" in step:
                require(PIN.fullmatch(step["uses"]) is not None, "npm actions must be pinned")
    preflight = next(s["run"] for s in workflow["jobs"]["release-contract"]["steps"] if s.get("id") == "release")
    for guard in ('test "$GITHUB_REPOSITORY" = lucasbabur/cutokyo',
                  'test "$GITHUB_REF" = "refs/tags/v$version"', 'git rev-parse HEAD',
                  'gh release view', '.isDraft == false'):
        require(guard in preflight, "npm publication requires official immutable public release")
    for job_name, step_name in (("cli-artifacts", "Reuse published native bytes for npm publication"),
                                ("global-installers", "Reuse the published npm tarball without repacking")):
        reuse = next(s for s in workflow["jobs"][job_name]["steps"] if s.get("name") == step_name)
        require(reuse.get("if") == "inputs.mode == 'publish-npm'" and "gh release download" in reuse.get("run", ""),
                "npm publication must reuse the published bytes")
    native_steps = workflow["jobs"]["cli-artifacts"]["steps"]
    require(any("smoke-npm.py" in s.get("run", "") and s.get("if") == "runner.os != 'Linux'"
                for s in native_steps), "npm wrapper must be exercised on macOS and Windows")
    job = workflow["jobs"]["npm-release"]
    require(set(job["needs"]) == {"release-contract", "cli-artifacts", "global-installers"},
            "npm must depend only on CLI artifacts")
    require(job["permissions"].get("id-token") == "write", "npm OIDC permission missing")
    steps = job["steps"]
    smoke = next(i for i, s in enumerate(steps) if "smoke-npm.py" in s.get("run", ""))
    require(any("kernel.apparmor_restrict_unprivileged_userns=0" in s.get("run", "") for s in steps[:smoke]),
            "npm smoke requires the Ubuntu namespace fix")
    assets = next(i for i, s in enumerate(steps) if "publish-cli-assets.py" in s.get("run", ""))
    publish = [i for i, s in enumerate(steps) if "npm publish" in s.get("run", "")]
    require(len(publish) == 1 and smoke < assets < publish[0], "test and publish native assets before npm")
    require(steps[assets].get("if") == "needs.release-contract.outputs.publish == 'true'",
            "asset publication must be gated")
    require(steps[publish[0]].get("if") == "needs.release-contract.outputs.publish == 'true' && inputs.mode == 'publish-npm'",
            "npm publication must be explicitly gated")


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
        lambda c, r: next(s for s in r["jobs"]["release-manifest"]["steps"] if "kernel.apparmor" in s.get("run", "")).update(run="true"),
        lambda c, r: c["jobs"]["build-test"]["strategy"]["matrix"].update(os=["ubuntu-latest"]),
        lambda c, r: r["jobs"]["tauri-artifacts"]["strategy"]["matrix"]["include"][1].update(target="x86_64-apple-darwin"),
        lambda c, r: r["jobs"]["release-contract"]["steps"][0].update(uses="actions/checkout@main"),
        lambda c, r: next(s for s in r["jobs"]["release-manifest"]["steps"] if "gh release create" in s.get("run", "")).pop("if"),
        lambda c, r: next(s for s in r["jobs"]["tauri-artifacts"]["steps"] if s.get("uses", "").startswith("tauri-apps/tauri-action@"))["with"].update(tauriScript="pnpm exec tauri"),
        lambda c, r: next(s for s in r["jobs"]["tauri-artifacts"]["steps"] if s.get("uses", "").startswith("tauri-apps/tauri-action@"))["with"].update(includeDebug="true"),
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
    npm = yaml.load((args.root / ".github/workflows/release-npm.yml").read_text(), Loader=yaml.BaseLoader)
    validate_npm(npm)
    npm_mutations = [
        lambda n: next(s for s in n["jobs"]["global-installers"]["steps"] if s.get("name") == "Reuse the published npm tarball without repacking").update(run="npm pack"),
        lambda n: n["on"]["workflow_dispatch"]["inputs"]["mode"].update(default="publish-npm"),
        lambda n: n["jobs"]["npm-release"].update(needs=["tauri-artifacts"]),
        lambda n: n["jobs"]["npm-release"]["permissions"].pop("id-token"),
        lambda n: next(s for s in n["jobs"]["npm-release"]["steps"] if "npm publish" in s.get("run", "")).pop("if"),
        lambda n: next(s for s in n["jobs"]["npm-release"]["steps"] if "kernel.apparmor" in s.get("run", "")).update(run="true"),
        lambda n: next(s for s in n["jobs"]["release-contract"]["steps"] if s.get("id") == "release").update(run="echo publish=true"),
        lambda n: n["jobs"]["npm-release"]["steps"][0].update(uses="actions/checkout@main"),
    ]
    for mutation in npm_mutations:
        bad = copy.deepcopy(npm)
        mutation(bad)
        try:
            validate_npm(bad)
        except ValueError:
            pass
        else:
            raise ValueError("npm workflow mutation escaped detection")
    workflows.append(npm)
    count = parse_powershell(workflows) if args.powershell else 0
    print(json.dumps({"workflow_documents": 3, "rejected_mutations": len(mutations) + len(npm_mutations),
                      "powershell_steps_parsed": count, "remote_jobs_executed": False}))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, KeyError, TypeError, yaml.YAMLError) as error:
        print(f"workflow-contract: {error}", file=sys.stderr)
        raise SystemExit(1)

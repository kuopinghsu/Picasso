#!/usr/bin/env python3
"""Select CI jobs from the complete push/PR diff; tags and manual runs validate all.

Use git rather than GitHub's path filters so large diffs, deletions and renames
are included, and keep the workflow running to provide a final required check.
An unavailable comparison or an unknown input selects full validation.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import sys


JOBS = ("rust", "wordpress", "deny", "guide", "windows", "linux", "appimage", "macos", "bench")
FULL_INPUTS = {
    ".github/workflows/ci.yml", "scripts/ci-changes.py", "scripts/test_ci_changes.py",
    "Makefile", "build.rs", "rust-toolchain", "rust-toolchain.toml",
    "scripts/release-notes.sh",
}
NOTICE_INPUTS = {"scripts/third-party-notices.py"}


def select(paths, full=False):
    """Return a conservative job plan without accessing git or the environment."""
    paths = set(paths)
    full = full or bool(paths & FULL_INPUTS) or any(p.startswith(".cargo/") for p in paths)
    plan = dict.fromkeys(JOBS, False)
    for path in paths:
        if path in FULL_INPUTS or path.startswith(".cargo/"):
            continue
        if path in {"Cargo.toml", "Cargo.lock", "deny.toml"}:
            plan["deny"] = True
            if path != "deny.toml":
                plan["rust"] = True
        elif path.startswith(("src/", "tests/", "benches/")) or path in {
            "build/world.bin", "build/picasso.ico", "images/banner.jpg",
        }:
            plan["rust"] = True
            if path == "src/bench.rs":
                plan["bench"] = True
            if path == "build/picasso.ico":
                plan["windows"] = True
            if path == "images/banner.jpg":
                plan["guide"] = True
        elif path.startswith("wordpress/") and path.endswith(".md"):
            pass
        elif path.startswith("wordpress/") or path == "scripts/build-theme.py":
            plan["wordpress"] = True
            # Rust inline-formatting tests consume this stylesheet directly.
            if path == "wordpress/voyagelight/assets/css/theme.css":
                plan["rust"] = True
        elif path.startswith("build/licenses/") or path in NOTICE_INPUTS:
            for job in ("windows", "linux", "appimage", "macos"):
                plan[job] = True
        elif path in {"build/picasso.desktop", "build/picasso.png"}:
            plan["linux"] = plan["appimage"] = True
            if path.endswith(".png"):
                plan["macos"] = True
        elif path == "build/AppRun" or path.startswith("docker/appimage/") or path in {
            "scripts/package-appimage.sh", "scripts/check-appimage.sh", "scripts/dnf-install.sh",
        }:
            plan["appimage"] = True
        elif path == "scripts/self-signed-identity.sh":
            plan["macos"] = True
        elif path.startswith("docker/guide-pdf/") or path in {
            "scripts/docs-pdf.sh", "scripts/guide-fonts.sh",
        } or (path.startswith("docs/") and path.endswith((".adoc", "theme.yml"))):
            plan["guide"] = True
        elif path.startswith("images/"):
            plan["guide"] = True
        elif path in {"scripts/check-docs.py", "scripts/make-help.py", ".gitignore", "LICENSE"}:
            pass  # Documentation/version checks always run.
        elif path.startswith(("docs/", ".github/")) or path.endswith((".md", ".txt", ".pdf")):
            pass
        else:
            full = True  # A new build input must not silently bypass verification.
    return {"full": full, **{job: full or value for job, value in plan.items()}}


def git(*args):
    return subprocess.check_output(["git", *args], stderr=subprocess.PIPE)


def changed_paths(event_name, event, head):
    """Pushes use their before SHA; PRs use the base/head merge-base."""
    base = (event.get("pull_request", {}).get("base", {}).get("sha")
            if event_name == "pull_request" else event.get("before"))
    for revision in (base, head):
        if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-fA-F]{40,64}", revision) or not revision.strip("0"):
            raise ValueError("missing or invalid comparison revision")
    if event_name == "pull_request":
        base = git("merge-base", base, head).decode().strip()
    return git("diff", "--name-only", "--no-renames", "-z", base, head, "--").decode(
        "utf-8", errors="surrogateescape"
    ).split("\0")[:-1]


def main():
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    full = event_name == "workflow_dispatch" or os.environ.get("GITHUB_REF", "").startswith("refs/tags/")
    paths = []
    if not full:
        try:
            if event_name not in {"push", "pull_request"}:
                raise ValueError("unknown event")
            event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
            head = os.environ.get("SOURCE_COMMIT") or event.get("after") or event.get("pull_request", {}).get("head", {}).get("sha")
            paths = changed_paths(event_name, event, head)
        except (OSError, KeyError, ValueError, subprocess.CalledProcessError) as error:
            print(f"Unable to compare changes; selecting full validation: {error}", file=sys.stderr)
            full = True
    plan = select(paths, full)
    output = "".join(f"{key}={str(value).lower()}\n" for key, value in plan.items())
    print(output, end="")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            stream.write(output)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            chosen = ", ".join(job for job in JOBS if plan[job]) or "documentation/version checks only"
            stream.write(f"### CI selection\n\nChanged files: {len(paths)}. Full validation: {plan['full']}.\n\nSelected: {chosen}.\n")


if __name__ == "__main__":
    main()

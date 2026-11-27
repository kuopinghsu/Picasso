#!/usr/bin/env python3
"""Select daily CI jobs since the last successful daily run; tags/manual runs validate all.

Daily runs only check: they never build packages. Release tags and manual runs
build and test every package.

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
import urllib.parse
import urllib.request


JOBS = ("rust", "wordpress", "deny", "guide", "windows", "linux", "appimage", "macos", "bench")
FULL_INPUTS = {
    ".github/workflows/ci.yml", "scripts/ci-changes.py", "scripts/test_ci_changes.py",
    "Makefile", "build.rs", "rust-toolchain", "rust-toolchain.toml",
    "scripts/release-notes.sh",
}
NOTICE_INPUTS = {"scripts/third-party-notices.py"}
# Jobs that build installable packages, which only manual runs and release tags do.
PACKAGE_JOBS = ("windows", "linux", "appimage", "macos")


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


def changed_paths(base, head):
    """Compare the last successfully checked daily snapshot with today's head."""
    for revision in (base, head):
        if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-fA-F]{40,64}", revision) or not revision.strip("0"):
            raise ValueError("missing or invalid comparison revision")
    try:
        git("cat-file", "-e", base + "^{commit}")
    except subprocess.CalledProcessError:
        # A two-snapshot comparison needs no complete history. Fetch only
        # the previous successful snapshot when checkout's shallow clone lacks it.
        git("fetch", "--no-tags", "--depth=1", "origin", base)
    return git("diff", "--name-only", "--no-renames", "-z", base, head, "--").decode(
        "utf-8", errors="surrogateescape"
    ).split("\0")[:-1]


def scheduled_base(repository, branch, token, api_url="https://api.github.com"):
    """Failed/cancelled runs never advance the last validated snapshot."""
    if not repository or not branch or not token:
        raise ValueError("missing repository, branch or token for the scheduled baseline")
    query = urllib.parse.urlencode({"event": "schedule", "status": "success", "branch": branch, "per_page": 1})
    url = f"{api_url.rstrip('/')}/repos/{repository}/actions/workflows/ci.yml/runs?{query}"
    request = urllib.request.Request(url, headers={
        "Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "Picasso-CI",
    })
    with urllib.request.urlopen(request, timeout=15) as response:
        runs = json.load(response).get("workflow_runs", [])
    for run in runs:
        if run.get("conclusion") == "success" and run.get("event") == "schedule" and run.get("head_branch") == branch:
            return run.get("head_sha")
    return None


def main():
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    full = event_name == "workflow_dispatch" or os.environ.get("GITHUB_REF", "").startswith("refs/tags/")
    paths = []
    if not full:
        try:
            if event_name != "schedule":
                raise ValueError("unknown event")
            event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text())
            repository = os.environ.get("GITHUB_REPOSITORY", "")
            branch = event.get("repository", {}).get("default_branch") or os.environ.get("GITHUB_REF_NAME", "")
            base = scheduled_base(repository, branch, os.environ.get("GH_TOKEN", ""),
                                  os.environ.get("GITHUB_API_URL", "https://api.github.com"))
            paths = changed_paths(base, os.environ.get("SOURCE_COMMIT") or os.environ.get("GITHUB_SHA"))
        except (OSError, KeyError, ValueError, TypeError, AttributeError, subprocess.CalledProcessError) as error:
            print(f"Unable to compare changes; selecting full validation: {error}", file=sys.stderr)
            full = True
    plan = select(paths, full)
    if event_name == "schedule":
        plan["deny"] = True  # New advisories do not require a repository change.
        # Daily runs check and never package, even when every check is selected:
        # the Linux x64 job tests the Rust code instead of the package jobs.
        for job in PACKAGE_JOBS:
            plan[job] = False
    output = "".join(f"{key}={str(value).lower()}\n" for key, value in plan.items())
    print(output, end="")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as stream:
            stream.write(output)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            chosen = ", ".join(job for job in JOBS if plan[job]) or "documentation/version checks only"
            stream.write(f"### CI selection\n\nChanged files: {len(paths)}. Full validation: {plan['full']}.\n\nSelected: {chosen}.\n")
            if event_name == "schedule":
                stream.write("\nPackages are built only by manual runs and release tags.\n")


if __name__ == "__main__":
    main()

"""What `make help` prints: each target with its `##` text, under the `##@`
sections, then the variables the targets take. In Python rather than awk and
echo, so it works where make runs recipes with cmd.exe, as on Windows.

    python3 scripts/make-help.py Makefile VERSION
"""
import re
import sys

VARIABLES = [
    ("ARGS=...", 'photo folders for run and run-stats: make run ARGS="~/Pictures"'),
    ("T=...", "only the tests whose name contains it: make test T=backup"),
    ("CORPUS=dir", "folder of your own photos for make corpus"),
    ("PREFIX=dir", "where install puts Picasso (default ~/.local)"),
    ("SIGN_IDENTITY", "macOS code-signing identity for builds (default Picasso Development)"),
    ("PICASSO_BENCH_PHOTOS, _TRIP, _FILES", "sizes for make bench (docs/PERFORMANCE.md)"),
    ("DOCS=...", "the .adoc files for make user-guide-pdf (default docs/*.adoc)"),
    ("DOCKER=...", "the container tool for user-guide-pdf and appimage-docker (default docker; podman works)"),
    ("APPIMAGE_PLATFORM=...", "processor for make appimage-docker: linux/amd64 (default) or linux/arm64"),
    ("VERSION=...", "version in the file name for make appimage-docker (default Cargo.toml's)"),
    ("RUST_VERSION=...", "Rust toolchain in the appimage-docker image (default 1.100.0)"),
]
TARGET = re.compile(r"^([a-z][a-z0-9-]*):.*?## (.*)$")


def main(makefile, version):
    print(f"Picasso {version}. Usage: make [target] [VARIABLE=value]")
    with open(makefile, encoding="utf-8") as lines:
        for line in lines:
            line = line.rstrip("\n")
            if line.startswith("##@ "):
                print(f"\n{line[4:]}")
            elif match := TARGET.match(line):
                print(f"  {match.group(1):<14} {match.group(2)}")
    print("\nVariables")
    for name, text in VARIABLES:
        print(f"  {name:<14} {text}" if len(name) <= 14 else f"  {name}   {text}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "")

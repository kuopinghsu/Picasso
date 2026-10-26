#!/usr/bin/env python3
"""Writes THIRD-PARTY-NOTICES.txt: the license of every crate built into
Picasso, with the license texts the crates publish.

    python3 scripts/third-party-notices.py [--target TRIPLE]... [-o FILE]

Only crates that end up in the program count, not those of tests or build
scripts. --target limits them to the platforms given (the host by default);
CI passes the platform it packages. A crate that publishes no license text
gets the standard text of its license from build/licenses, and the script
fails when that text is missing, so a new dependency can't be left out.

Runs on Python 3.6 (AlmaLinux 8) and newer.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
STANDARD_TEXTS = os.path.join(ROOT, "build", "licenses")
LICENSE_NAME = re.compile(r"^(licen[sc]e|copying|copyright|notice|unlicense|ofl|ufl)", re.I)


def run(args):
    out = subprocess.run(args, cwd=ROOT, stdout=subprocess.PIPE, check=True)
    return out.stdout.decode("utf-8")


def host_target():
    for line in run(["rustc", "-vV"]).splitlines():
        if line.startswith("host: "):
            return line[len("host: "):]
    sys.exit("rustc -vV printed no host")


def shipped_packages(target):
    """Packages reached from Picasso through normal dependencies on `target`."""
    meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target]))
    packages = {package["id"]: package for package in meta["packages"]}
    nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen = set()
    todo = [root]
    while todo:
        current = todo.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes[current]["deps"]:
            if any(kind["kind"] is None for kind in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    seen.discard(root)
    return packages[root], [packages[id] for id in seen]


def license_files(package):
    """License texts in the crate's folder, and beside the fonts it carries."""
    folder = os.path.dirname(package["manifest_path"])
    found = []
    for current, dirs, files in os.walk(folder):
        depth = os.path.relpath(current, folder).count(os.sep) + (current != folder)
        dirs[:] = [name for name in dirs if depth < 2 and name not in ("src", "tests", "benches", "examples", "target")]
        fonts = any(name.lower().endswith((".ttf", ".otf")) for name in files)
        for name in sorted(files):
            if LICENSE_NAME.match(name) or (fonts and name.lower().endswith(".txt")):
                path = os.path.join(current, name)
                if os.path.getsize(path) < 200000:
                    found.append(path)
    if package.get("license_file"):
        path = os.path.join(folder, package["license_file"])
        if os.path.isfile(path) and path not in found:
            found.append(path)
    return [(os.path.relpath(path, folder), path) for path in found]


def license_ids(expression):
    words = re.findall(r"[A-Za-z0-9.+-]+", expression or "")
    return sorted({word for word in words if word not in ("OR", "AND", "WITH", "LLVM-exception")})


def read(path):
    with open(path, encoding="utf-8", errors="replace") as file:
        return file.read().replace("\r\n", "\n").strip() + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--target", action="append", help="platform to list the crates of (repeatable)")
    parser.add_argument("-o", "--output", help="file to write instead of standard output")
    options = parser.parse_args()

    picasso = None
    crates = {}
    for target in options.target or [host_target()]:
        picasso, packages = shipped_packages(target)
        for package in packages:
            crates[package["id"]] = package
    crates = sorted(crates.values(), key=lambda package: (package["name"], package["version"]))

    out = []
    out.append("Third-party software in Picasso {}\n".format(picasso["version"]))
    out.append(
        "Picasso is released under the MIT License (LICENSE). It is built from the\n"
        "Rust crates below, each under the license shown. Where a crate offers a\n"
        "choice of licenses, Picasso uses it under any one of them.\n"
    )
    rawler = [package for package in crates if package["name"] == "rawler"]
    if rawler:
        out.append(
            "rawler, which reads camera RAW files, is under the GNU Lesser General Public\n"
            "License 2.1. Its source, version {0}, comes with each Picasso release on\n"
            "GitHub as rawler-{0}-source.tar.gz, and is also at\n"
            "https://crates.io/crates/rawler/{0} and {1}.\n"
            "Picasso's complete source is at {2} so that\n"
            "Picasso can be rebuilt with a changed rawler, as that license allows.\n".format(
                rawler[0]["version"], rawler[0]["repository"] or "its authors' repository", picasso["repository"]
            )
        )
    for package in crates:
        if license_ids(package["license"]) == ["MPL-2.0"]:
            out.append(
                "{0} is under the Mozilla Public License 2.0 and is used unchanged. Its\n"
                "source, version {1}, is at https://crates.io/crates/{0}/{1}\n"
                "and {2}.\n".format(package["name"], package["version"], package["repository"] or "its authors' repository")
            )
    out.append("The country borders of the Travel Map are public-domain Natural Earth data.\n")

    out.append("\n== Crates ==\n")
    width = max(len(package["name"]) + len(package["version"]) + 1 for package in crates)
    for package in crates:
        label = "{} {}".format(package["name"], package["version"])
        out.append("{}  {}".format(label.ljust(width), package["license"] or "(no license stated)"))

    out.append("\n\n== License texts published with the crates ==\n")
    printed = {}
    standard_needed = set()
    for package in crates:
        label = "{} {}".format(package["name"], package["version"])
        files = license_files(package)
        out.append("\n---- {} ({}) ----\n".format(label, package["license"]))
        if not files:
            ids = license_ids(package["license"])
            authors = ", ".join(package.get("authors") or []) or "the {} authors".format(package["name"])
            out.append(
                "The crate publishes no license file. Copyright (c) {}. It is under\n"
                "{}; the standard text of each is at the end.\n".format(authors, package["license"])
            )
            standard_needed.update(ids)
            continue
        for name, path in files:
            text = read(path)
            digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
            if digest in printed:
                out.append("{}: the same text as {} above.\n".format(name, printed[digest]))
            else:
                printed[digest] = "{} of {}".format(name, label)
                out.append("{}:\n\n{}".format(name, text))

    missing = [id for id in sorted(standard_needed) if not os.path.isfile(os.path.join(STANDARD_TEXTS, id + ".txt"))]
    if missing:
        sys.exit("No standard text in build/licenses for: {}".format(", ".join(missing)))
    out.append("\n\n== Standard license texts ==\n")
    for id in sorted(standard_needed):
        out.append("\n---- {} ----\n\n{}".format(id, read(os.path.join(STANDARD_TEXTS, id + ".txt"))))

    text = "\n".join(out)
    if options.output:
        with open(options.output, "w", encoding="utf-8", newline="\n") as file:
            file.write(text)
        print("Wrote {} ({} crates)".format(options.output, len(crates)), file=sys.stderr)
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main()

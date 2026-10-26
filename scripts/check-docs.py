#!/usr/bin/env python3
"""Checks that the documentation still matches the code: run by CI and
`make check-docs`.

- Every link and image to a file in the repository points at one that exists.
- Every <<anchor>> in an AsciiDoc file has its [#anchor].
- Every `make` target and `scripts/` file the documents name exists.
- Every PICASSO_* setting the documents name is read somewhere in src/.
- The version in Cargo.toml has a section in CHANGELOG.md, and every
  section has its comparison link at the bottom.

The two READMEs, and the two user guides, are compared with each other by
`cargo test` (the_two_readmes_say_the_same_things and
the_two_user_guides_say_the_same_things).
"""

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as file:
        return file.read()


def documents():
    paths = glob.glob(os.path.join(ROOT, "*.md")) + glob.glob(os.path.join(ROOT, "docs", "*.md"))
    paths += glob.glob(os.path.join(ROOT, "docs", "*.adoc"))
    return sorted(os.path.relpath(path, ROOT) for path in paths)


# ``` blocks in Markdown, and ---- blocks in AsciiDoc.
BLOCK = r"^```[^\n]*\n(.*?)^```|^----\n(.*?)^----$"


def code(text):
    """The `code` spans and the lines of code blocks."""
    blocks = [a or b for a, b in re.findall(BLOCK, text, flags=re.S | re.M)]
    spans = re.findall(r"`([^`\n]+)`", re.sub(BLOCK, "", text, flags=re.S | re.M))
    return spans + [line.strip() for block in blocks for line in block.splitlines()]


def linked_paths(doc, text):
    """The repository paths a document links to, as written and as paths
    from the top of the repository."""
    folder = os.path.dirname(doc)
    found = re.findall(r"\]\(([^)\s]+)\)", text)
    if doc.endswith(".adoc"):
        found += re.findall(r"(?<![A-Za-z])(?:link|xref):([^\s\[]+)\[", text)
        images = re.search(r"^:imagesdir:\s*(\S+)", text, flags=re.M)
        for image in re.findall(r"^image::([^\[]+)\[", text, flags=re.M):
            found.append(os.path.join(images.group(1), image) if images else image)
    for target in found:
        if re.match(r"[a-z]+:", target) or target.startswith("#"):
            continue
        yield target, os.path.normpath(os.path.join(folder, target.split("#")[0]))


def main():
    problems = []
    makefile = read("Makefile")
    targets = set(re.findall(r"^([A-Za-z][A-Za-z0-9_-]*):", makefile, flags=re.M))
    sources = ""
    for path in glob.glob(os.path.join(ROOT, "src", "**", "*.rs"), recursive=True):
        with open(path, encoding="utf-8") as file:
            sources += file.read()

    for doc in documents():
        text = read(doc)
        for target, path in linked_paths(doc, text):
            if not os.path.exists(os.path.join(ROOT, path)):
                problems.append("{}: the link to {} leads nowhere".format(doc, target))
        anchors = set(re.findall(r"^\[#([\w-]+)\]", text, flags=re.M))
        for anchor in re.findall(r"<<([\w-]+)(?:,[^>]*)?>>", text) if doc.endswith(".adoc") else []:
            if anchor not in anchors:
                problems.append("{}: <<{}>> has no [#{}]".format(doc, anchor, anchor))
        for snippet in code(text):
            for target in re.findall(r"(?:^|[;&|]\s*)make ([a-z][a-z0-9-]*)", snippet):
                if target not in targets:
                    problems.append("{}: `make {}` is not in the Makefile".format(doc, target))
            for script in re.findall(r"scripts/[A-Za-z0-9_.-]+", snippet):
                if not os.path.exists(os.path.join(ROOT, script)):
                    problems.append("{}: {} does not exist".format(doc, script))
        for name in sorted(set(re.findall(r"PICASSO_[A-Z_]+", text))):
            if '"{}"'.format(name) not in sources:
                problems.append("{}: src/ never reads {}".format(doc, name))

    version = re.search(r'^version\s*=\s*"([^"]+)"', read("Cargo.toml"), flags=re.M).group(1)
    changelog = read("CHANGELOG.md")
    sections = re.findall(r"^## \[([^\]]+)\]", changelog, flags=re.M)
    links = set(re.findall(r"^\[([^\]]+)\]: http", changelog, flags=re.M))
    if version not in sections:
        problems.append("CHANGELOG.md has no section for {}, the version in Cargo.toml".format(version))
    for section in sections:
        if section not in links:
            problems.append("CHANGELOG.md has no [{}]: link at the bottom".format(section))

    if problems:
        print("\n".join(problems))
        sys.exit(1)
    print("Documentation checked: {} files".format(len(documents())))


if __name__ == "__main__":
    main()

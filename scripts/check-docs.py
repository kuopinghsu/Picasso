#!/usr/bin/env python3
"""Checks that the documentation still matches the code: run by CI and
`make check-docs`.

- Every link and image to a file in the repository points at one that exists.
- Every <<anchor>> and xref:anchor[] in an AsciiDoc file leads to an ID it defines.
- Every `make` target and `scripts/` file the documents name exists.
- Every PICASSO_* setting the documents name is read somewhere in src/.
- The version in Cargo.toml has a section in CHANGELOG.md, and every
  section has its comparison link at the bottom.

The bilingual READMEs and guides are also compared here, so documentation-only
CI retains their structure, command and shortcut checks without compiling Rust.
Traditional Chinese wording rules are read from the existing Rust test constants.
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
    paths += glob.glob(os.path.join(ROOT, "wordpress", "README*.md"))
    return sorted(os.path.relpath(path, ROOT) for path in paths)


# ``` blocks in Markdown, and ---- blocks in AsciiDoc.
MARKDOWN_BLOCK = r"^```[^\n]*\n(.*?)^```"
ASCIIDOC_BLOCK = r"^----\n(.*?)^----$"


def code(doc, text):
    """The `code` spans and the lines of code blocks."""
    block = ASCIIDOC_BLOCK if doc.endswith(".adoc") else MARKDOWN_BLOCK
    blocks = re.findall(block, text, flags=re.S | re.M)
    spans = re.findall(r"`([^`\n]+)`", re.sub(block, "", text, flags=re.S | re.M))
    return spans + [line.strip() for lines in blocks for line in lines.splitlines()]


def linked_paths(doc, text):
    """The repository paths a document links to, as written and as paths
    from the top of the repository."""
    folder = os.path.dirname(doc)
    found = re.findall(r"\]\(([^)\s]+)\)", text)
    if doc.endswith(".adoc"):
        found += re.findall(r"(?<![A-Za-z])link:([^\s\[]+)\[", text)
        # xref:name[] without .adoc is an anchor in the same document.
        found += [t for t in re.findall(r"(?<![A-Za-z])xref:([^\s\[]+)\[", text) if ".adoc" in t]
        images = re.search(r"^:imagesdir:\s*(\S+)", text, flags=re.M)
        for image in re.findall(r"^image::([^\[]+)\[", text, flags=re.M):
            found.append(os.path.join(images.group(1), image) if images else image)
    for target in found:
        if re.match(r"[a-z]+:", target) or target.startswith("#"):
            continue
        yield target, os.path.normpath(os.path.join(folder, target.split("#")[0]))


def anchors(text):
    """The IDs an AsciiDoc document defines: [#id], [[id]], and the ones
    Asciidoctor makes from section titles, such as _getting_help."""
    text = re.sub(ASCIIDOC_BLOCK, "", text, flags=re.S | re.M)
    found = set(re.findall(r"^\[#([\w-]+)", text, flags=re.M))
    found |= set(re.findall(r"\[\[([\w-]+)(?:,[^\]]*)?\]\]", text))
    # The document title, a single =, gets no ID, and neither does a
    # section whose [#id] line replaces it.
    for explicit, title in re.findall(r"^(\[#.*\n)?={2,6} (.+)$", text, flags=re.M):
        if explicit:
            continue
        title = re.sub(r"[^ \w.-]", "", title.lower())
        found.add("_" + re.sub(r"[ .-]+", "_", title).strip("_"))
    return found


def anchor_refs(text):
    """The anchors an AsciiDoc document links to within itself."""
    refs = re.findall(r"<<([\w-]+)(?:,[^>]*)?>>", text)
    return refs + re.findall(r"(?<![A-Za-z])xref:([\w-]+)\[", text)


def outline(text, asciidoc=False):
    """The same bilingual document contract checked by src/i18n.rs."""
    shape, spans = [], []
    in_code = False
    for line in text.splitlines():
        fence = line == "----" if asciidoc else line.startswith("```")
        if fence:
            in_code = not in_code
            shape.append("code block")
            continue
        if in_code:
            continue
        if line.startswith("=" if asciidoc else "#"):
            shape.append(line.split(" ")[0])
        elif asciidoc and line.startswith("[#"):
            shape.append(line)
        elif line.startswith(("* ", ". ") if asciidoc else "- "):
            shape.append("list item")
        elif asciidoc and line.endswith("::"):
            shape.append("term")
        elif asciidoc and line == "|===":
            shape.append("table")
        elif line.startswith("|"):
            shape.append("table row of {} cells".format(line.count("|") - (not asciidoc)))
        elif asciidoc and line.startswith("image::"):
            shape.append("image")
        elif asciidoc and line.startswith("NOTE:"):
            shape.append("note")
        spans.extend(line.split("`")[1::2])
        if asciidoc:
            spans.extend("kbd:[" + key + "]" for key in re.findall(r"kbd:\[([^]]*)\]", line))
    return shape, sorted(spans)


def bilingual_problems():
    problems = []
    for en, zh, asciidoc in [("README.md", "README_zh-TW.md", False),
                              ("docs/USER_GUIDE.adoc", "docs/USER_GUIDE_zh-TW.adoc", True)]:
        if outline(read(en), asciidoc) != outline(read(zh), asciidoc):
            problems.append(f"{en} and {zh}: sections, lists, code spans or shortcuts differ")
    rules = read("src/i18n.rs")
    terms = re.search(r"const ZH_HANT_TERMS\b.*?=\s*&\[(.*?)\];", rules, re.S)
    simplified = re.search(r'const SIMPLIFIED_ONLY[^=]*=\s*"([^"\n]*)"', rules)
    if not terms or not simplified:
        return problems + ["Could not read Traditional Chinese wording rules from src/i18n.rs"]
    terms = re.findall(r'\("([^"]+)",\s*"([^"]+)"\)', terms.group(1))
    if not terms:
        return problems + ["No Traditional Chinese wording rules were found in src/i18n.rs"]
    for name in ["README_zh-TW.md", "docs/USER_GUIDE_zh-TW.adoc"]:
        for number, line in enumerate(read(name).splitlines(), 1):
            for word, standard in terms:
                if word in line:
                    problems.append(f"{name}:{number}: {word} should be {standard}")
            if any(char in simplified.group(1) for char in line):
                problems.append(f"{name}:{number}: unexpected Simplified Chinese character")
    return problems


def main():
    problems = bilingual_problems()
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
        if doc.endswith(".adoc"):
            defined = anchors(text)
            for anchor in anchor_refs(text):
                if anchor not in defined:
                    problems.append("{}: the link to #{} leads nowhere".format(doc, anchor))
        for snippet in code(doc, text):
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

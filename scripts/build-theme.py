#!/usr/bin/env python3
"""Builds the optional VoyageLight WordPress theme and the Picasso Places
plugin as install ZIPs.

    build-theme.py [--check | --sync] [OUT_DIR]

Compiles each languages/*.po to a .mo beside it in the ZIP, after checking
that every text the PHP files pass to a translation function is in each
.po. The ZIP is the same, byte for byte, for the same files: names sorted,
dates fixed, no file-system metadata. --check stops after the checks.

The plugin carries copies of the theme's location reader, editor scripts and
map script (SHARED). The theme's files are the source: the checks fail when
a copy differs, and --sync rewrites the copies from the theme, then checks
without building. The PHP reader
is put in the plugin's namespace, so its functions don't clash with the
theme's when both are installed.
"""

import re
import struct
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
THEME = ROOT / "wordpress" / "voyagelight"
PLUGIN = ROOT / "wordpress" / "picasso-places"
SHARED = ["inc/geo.php", "assets/js/geo.js", "assets/js/geo-admin.js", "assets/js/map.js", "assets/css/geo-admin.css"]
NAMESPACE = "PicassoPlaces"
# 1980-01-01, the earliest date a ZIP can hold.
DATE = (1980, 1, 1, 0, 0, 0)
CALL = re.compile(r"\b(?:__|_e|esc_html__|esc_html_e|esc_attr__|esc_attr_e|_n)\(\s*'((?:[^'\\]|\\.)*)'")
# WordPress shows these plugin headers translated with the plugin's text domain.
HEADER = re.compile(r"^ \* (?:Plugin Name|Description):\s*(.+)$", re.M)


def php_files(folder):
    return [*folder.glob("*.php"), *folder.glob("inc/*.php")]


def php_texts(folder):
    texts = set()
    for path in php_files(folder):
        for text in CALL.findall(path.read_text(encoding="utf-8")):
            texts.add(text.replace("\\'", "'").replace("\\\\", "\\"))
    if folder == PLUGIN:
        texts.update(HEADER.findall((PLUGIN / "picasso-places.php").read_text(encoding="utf-8")))
    return texts


def unquote(chunk):
    parts = re.findall(r'"((?:[^"\\]|\\.)*)"', chunk)
    text = "".join(parts)
    return re.sub(r'\\(["\\n])', lambda m: {"n": "\n"}.get(m.group(1), m.group(1)), text)


def read_po(path):
    """The .po's messages: (msgid, msgid_plural or None, [msgstr...]), header first."""
    messages = []
    for block in path.read_text(encoding="utf-8").split("\n\n"):
        fields = {}
        for match in re.finditer(r'^(msgid_plural|msgid|msgstr(?:\[\d+\])?) ((?:".*"\n?)+)', block, re.M):
            fields[match.group(1)] = unquote(match.group(2))
        if "msgid" not in fields:
            continue
        strings = [fields[key] for key in sorted(k for k in fields if k.startswith("msgstr"))]
        messages.append((fields["msgid"], fields.get("msgid_plural"), strings))
    return messages


def mo_bytes(messages):
    """A GNU .mo file of `messages`, sorted by key as gettext expects."""
    pairs = []
    for msgid, plural, strings in messages:
        key = msgid if plural is None else msgid + "\0" + plural
        pairs.append((key.encode("utf-8"), "\0".join(strings).encode("utf-8")))
    pairs.sort()
    count = len(pairs)
    keys_at = 7 * 4
    values_at = keys_at + count * 8
    data_at = values_at + count * 8
    keys, values, data = [], [], b""
    for key, _ in pairs:
        keys.append((len(key), data_at + len(data)))
        data += key + b"\0"
    for _, value in pairs:
        values.append((len(value), data_at + len(data)))
        data += value + b"\0"
    head = struct.pack("<7I", 0x950412DE, 0, count, keys_at, values_at, 0, data_at)
    table = b"".join(struct.pack("<2I", *entry) for entry in keys + values)
    return head + table + data


def shared_copy(name):
    """The plugin's copy of the theme's file `name`."""
    data = (THEME / name).read_bytes()
    if name.endswith(".php"):
        head, docblock_end, rest = data.decode("utf-8").partition("\n */\n")
        note = f"\n// Made from wordpress/voyagelight/{name} by scripts/build-theme.py --sync; edit that file instead.\n"
        data = (head + docblock_end + note + f"namespace {NAMESPACE};\n" + rest).encode("utf-8")
    return data


def sync():
    for name in SHARED:
        target = PLUGIN / name
        data = shared_copy(name)
        if not target.exists() or target.read_bytes() != data:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            print(f"updated {target.relative_to(ROOT)}")


def check_translations(folder):
    texts = php_texts(folder)
    failed = False
    for po in sorted((folder / "languages").glob("*.po")):
        messages = read_po(po)
        known = {msgid for msgid, _, strings in messages if msgid and all(strings)}
        for text in sorted(texts - known):
            print(f"{po.relative_to(ROOT)}: no translation for {text!r}", file=sys.stderr)
            failed = True
        for msgid, _, strings in messages:
            if msgid and any(s.count("%") != msgid.count("%") for s in strings):
                print(f"{po.relative_to(ROOT)}: placeholders differ in {msgid!r}", file=sys.stderr)
                failed = True
    return texts, failed


def check_plugin():
    failed = False
    for name in SHARED:
        copy = PLUGIN / name
        if not copy.exists() or copy.read_bytes() != shared_copy(name):
            print(f"{copy.relative_to(ROOT)} differs from wordpress/voyagelight/{name}, its source: "
                  "edit the theme's file, then run python3 scripts/build-theme.py --sync", file=sys.stderr)
            failed = True
    for path in php_files(PLUGIN):
        if re.search(r",\s*'voyagelight'\s*\)", path.read_text(encoding="utf-8")):
            print(f"{path.relative_to(ROOT)}: uses the theme's text domain instead of 'picasso-places'", file=sys.stderr)
            failed = True
    # load_plugin_textdomain() reads only languages/<text domain>-<locale>.mo.
    pos = sorted((PLUGIN / "languages").glob("*.po"))
    for po in pos:
        if not po.name.startswith("picasso-places-"):
            print(f"{po.relative_to(ROOT)}: WordPress won't load it; name it picasso-places-<locale>.po", file=sys.stderr)
            failed = True
    if not pos:
        print("wordpress/picasso-places/languages: no .po file", file=sys.stderr)
        failed = True
    main_file = (PLUGIN / "picasso-places.php").read_text(encoding="utf-8")
    constant = re.search(r"^const VERSION\s*=\s*'([^']+)';", main_file, re.M)
    stable = re.search(r"^Stable tag:\s*(\S+)", (PLUGIN / "readme.txt").read_text(encoding="utf-8"), re.M)
    found = {constant.group(1) if constant else None, stable.group(1) if stable else None}
    if found != {plugin_version()}:
        print(f"wordpress/picasso-places: the Version header ({plugin_version()}), const VERSION and readme.txt's "
              "Stable tag must agree", file=sys.stderr)
        failed = True
    return failed


def check_day_bound():
    """The Journey day limit is the same number in the theme, its script, and Picasso."""
    php = re.search(r"^const VL_GEO_DAY_MAX = (\d+);", (THEME / "inc/geo.php").read_text(encoding="utf-8"), re.M)
    js = re.search(r"^\tconst DAY_MAX = (\d+);", (THEME / "assets/js/geo.js").read_text(encoding="utf-8"), re.M)
    rust = re.search(r"^const JOURNEY_DAY_MAX: i64 = (\d+);", (ROOT / "src" / "publish.rs").read_text(encoding="utf-8"), re.M)
    if php and js and rust and len({php.group(1), js.group(1), rust.group(1)}) == 1:
        return False
    print("VL_GEO_DAY_MAX, geo.js DAY_MAX and publish.rs JOURNEY_DAY_MAX must be the same number", file=sys.stderr)
    return True


def check():
    theme_texts, theme_failed = check_translations(THEME)
    plugin_texts, plugin_failed = check_translations(PLUGIN)
    plugin_failed = check_plugin() or plugin_failed
    if theme_failed or plugin_failed or check_day_bound():
        sys.exit(1)
    return theme_texts, plugin_texts


def theme_version():
    match = re.search(r"^Version:\s*(\S+)", (THEME / "style.css").read_text(encoding="utf-8"), re.M)
    return match.group(1)


def plugin_version():
    match = re.search(r"^ \* Version:\s*(\S+)", (PLUGIN / "picasso-places.php").read_text(encoding="utf-8"), re.M)
    return match.group(1)


def add(archive, name, data, executable=False):
    info = zipfile.ZipInfo(name, DATE)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.create_system = 3
    info.external_attr = (0o100755 if executable else 0o100644) << 16
    archive.writestr(info, data)


def build(source, slug, version, out_dir):
    out_dir.mkdir(parents=True, exist_ok=True)
    target = out_dir / f"{slug}-{version}.zip"
    files = {}
    for path in sorted(source.rglob("*")):
        if path.is_file() and path.name != ".DS_Store":
            files[path.relative_to(source).as_posix()] = path
    for po in (source / "languages").glob("*.po"):
        files[f"languages/{po.stem}.mo"] = mo_bytes(read_po(po))
    with zipfile.ZipFile(target, "w") as archive:
        for name in sorted(files):
            item = files[name]
            data = item if isinstance(item, bytes) else item.read_bytes()
            add(archive, f"{slug}/" + name, data, name.endswith(".sh"))
    return target


def main():
    args = sys.argv[1:]
    if "--sync" in args:
        sync()
    theme_texts, plugin_texts = check()
    if "--check" in args or "--sync" in args:
        print(f"VoyageLight: {len(theme_texts)} texts translated in every language")
        print(f"Picasso Places: {len(plugin_texts)} texts translated in every language, "
              f"{len(SHARED)} files shared with the theme in sync")
        return
    out_dir = Path(args[0]) if args else ROOT / "target" / "theme"
    print(build(THEME, "voyagelight", theme_version(), out_dir))
    print(build(PLUGIN, "picasso-places", plugin_version(), out_dir))


if __name__ == "__main__":
    main()

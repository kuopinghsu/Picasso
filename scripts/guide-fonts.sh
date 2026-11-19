#!/usr/bin/env bash
# Puts the fonts the User Guide's PDF themes (docs/guide-theme.yml) take
# characters from in DIR, default target/guide-fonts, for asciidoctor-pdf's
# pdf-fontsdir: Noto Sans TC for the Chinese text and DejaVu Sans for symbols
# such as ⌘ and ⇄. scripts/docs-pdf.sh runs it.
#
#     bash scripts/guide-fonts.sh [DIR]
#
# Noto Sans TC is downloaded once from Google Fonts, at the commit that last
# changed it, checked against its SHA-256, and cut into a regular and a bold
# font with fontTools, as asciidoctor-pdf can't pick a weight of a variable
# font. Needs curl, Python 3 with fontTools ($PYTHON, default python3), and
# DejaVu Sans, found with fc-match (fontconfig) or in /usr/share/fonts.
set -euo pipefail

fonts=${1:-target/guide-fonts}
python=${PYTHON:-python3}
mkdir -p "$fonts"

# name, commit of github.com/google/fonts, SHA-256 of the variable font.
noto=(
    "NotoSansTC b950a7257470b900078f2bf3223823a8602de7e1 864727d210d54f2537bbe23b3a839436c3992af72de9322af5270897246bd44f"
)
instance=(-m fontTools.varLib.instancer --update-name-table -q)

for entry in "${noto[@]}"; do
    read -r name commit sha256 <<<"$entry"
    # Written once both weights are made: fonts from another download, other
    # options or an interrupted run are made again.
    stamp=$fonts/$name.made
    made="$sha256 ${instance[*]}"
    if [ "$(cat "$stamp" 2>/dev/null)" = "$made" ]; then
        continue
    fi
    rm -f "$stamp"
    folder=$(echo "$name" | tr '[:upper:]' '[:lower:]')
    # Downloaded and cut in a folder of the machine's own, not in DIR, which
    # may be shared from another system (as Docker Desktop shares a Windows
    # folder) where a large file being written can come out short.
    work=$(mktemp -d)
    variable=$work/$name-variable.ttf
    echo "Downloading $name"
    curl -fsSL --retry 3 -o "$variable" "https://raw.githubusercontent.com/google/fonts/$commit/ofl/$folder/$name%5Bwght%5D.ttf"
    echo "$sha256  $variable" | sha256sum -c --quiet -
    "$python" "${instance[@]}" "$variable" wght=400 -o "$work/$name-Regular.ttf"
    "$python" "${instance[@]}" "$variable" wght=700 -o "$work/$name-Bold.ttf"
    cp "$work/$name-Regular.ttf" "$work/$name-Bold.ttf" "$fonts/"
    rm -r "$work"
    echo "$made" >"$stamp"
done

for file in DejaVuSans.ttf DejaVuSans-Bold.ttf; do
    if [ -f "$fonts/$file" ]; then
        continue
    fi
    found=$(fc-match -f '%{file}' "DejaVu Sans:style=$([ "$file" = DejaVuSans.ttf ] && echo Book || echo Bold)" 2>/dev/null || true)
    if [ "$(basename "${found:-none}")" != "$file" ]; then
        found=/usr/share/fonts/truetype/dejavu/$file
    fi
    if [ ! -f "$found" ]; then
        echo "error: $file not found; install DejaVu Sans (fonts-dejavu-core)" >&2
        exit 1
    fi
    cp "$found" "$fonts/"
done

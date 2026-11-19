#!/usr/bin/env bash
# Turns each docs/*.adoc into a PDF with asciidoctor-pdf, as
# `make user-guide-pdf` runs it inside the image in docker/guide-pdf and
# `make guide-pdf` runs it directly. Each PDF is written beside its .adoc, or
# in OUT with --out. A file whose name ends in _zh-TW takes the Traditional
# Chinese theme (docs/guide-zh-TW-theme.yml), any other the English one
# (docs/guide-theme.yml). Each is laid out as a book, with a title page
# showing the version from Cargo.toml and the month of the build, in the
# guide's language. A character no font has fails the build rather than
# printing as a box. scripts/guide-fonts.sh fetches the fonts first.
#
#     bash scripts/docs-pdf.sh [--out OUT] [docs/FILE.adoc ...]
set -euo pipefail

out=
if [ "${1:-}" = --out ]; then
    out=$2
    shift 2
    mkdir -p "$out"
fi
files=("$@")
if [ ${#files[@]} -eq 0 ]; then
    files=(docs/*.adoc)
fi

fonts=target/guide-fonts
bash scripts/guide-fonts.sh "$fonts"
version=${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)}

for file in "${files[@]}"; do
    case "$file" in
        *_zh-TW.adoc)
            theme=guide-zh-TW
            revdate=$(date +'%Y 年 %-m 月')
            ;;
        *)
            theme=guide
            revdate=$(LC_ALL=C date +'%B %Y')
            ;;
    esac
    dir=${out:-$(dirname "$file")}
    echo "$file -> $dir/$(basename "${file%.adoc}").pdf ($theme theme)"
    # --verbose, because asciidoctor-pdf reports a missing character only then.
    asciidoctor-pdf --verbose --failure-level=WARN -d book \
        -a pdf-themesdir=docs -a pdf-theme="$theme" \
        -a "pdf-fontsdir=$fonts;GEM_FONTS_DIR" \
        -a revnumber="$version" -a revdate="$revdate" \
        -D "$dir" "$file"
done

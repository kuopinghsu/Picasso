#!/usr/bin/env bash
# Checks the newest AppImage in dist/ for ARCH (amd64 or arm64) on the system
# it runs on, as `make appimage-docker-check` runs it in a bare AlmaLinux 8.4
# container: unpacked, Picasso and each bundled library must find every
# library and glibc symbol they need among the bundled ones and the system's.
# The same check as the AppImage startup job in .github/workflows/ci.yml.
#   bash scripts/check-appimage.sh [amd64|arm64]
set -euo pipefail

cd "$(dirname "$0")/.."

case "${1:-amd64}" in
    amd64 | x86_64 | x64) label=x64 ;;
    arm64 | aarch64) label=arm64 ;;
    *)
        echo "Unknown architecture $1; use amd64 or arm64" >&2
        exit 1
        ;;
esac

image=$(ls -t dist/picasso-*-linux-"$label".AppImage 2>/dev/null | head -n 1 || true)
if [ -z "$image" ]; then
    echo "No dist/picasso-*-linux-$label.AppImage; run 'make appimage-docker' first" >&2
    exit 1
fi
echo "Checking $image on $(cat /etc/redhat-release 2>/dev/null || uname -sr), $(ldd --version | head -n 1)"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$image" "$work/picasso.AppImage"
chmod +x "$work/picasso.AppImage"
(cd "$work" && ./picasso.AppImage --appimage-extract >/dev/null)

cd "$work/squashfs-root"
bundled=$(printf '%s:' "$PWD"/usr/lib/picasso/*)
failed=0
for file in usr/bin/picasso usr/lib/picasso/*/*; do
    if LD_LIBRARY_PATH=${bundled%:} ldd "$file" 2>&1 | grep 'not found'; then
        echo "$file needs the libraries above" >&2
        failed=1
    fi
done
if [ "$failed" = 0 ]; then
    echo "Picasso and its $(ls usr/lib/picasso | wc -l) bundled libraries find everything they need"
fi
exit $failed

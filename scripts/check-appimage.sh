#!/usr/bin/env bash
# Checks the newest AppImage in dist/ for ARCH (amd64 or arm64) on the system
# it runs on, as `make appimage-docker-check` runs it in AlmaLinux 8.4
# containers. Unpacked, Picasso and each bundled library must find every
# library and glibc symbol they need, the same check as the AppImage startup
# job in .github/workflows/ci.yml.
#
# With --system-libs, it first installs the runtime libraries the AppImage
# bundles from this release's own repositories (an older release's are on
# vault.almalinux.org), as a desktop has them. AppRun then uses the system's
# copies, so Picasso must work with those older libraries, which the bare
# check never tries. Needs root for dnf.
#   bash scripts/check-appimage.sh [--system-libs] [amd64|arm64]
set -euo pipefail

cd "$(dirname "$0")/.."

system_libs=
if [ "${1:-}" = --system-libs ]; then
    system_libs=1
    shift
fi

case "${1:-amd64}" in
    amd64 | x86_64 | x64) label=x64 abi=x86-64 ;;
    arm64 | aarch64) label=arm64 abi=AArch64 ;;
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

if [ -n "$system_libs" ]; then
    # An AlmaLinux release older than the newest is only on the vault.
    . /etc/os-release
    for repo in /etc/yum.repos.d/almalinux*.repo; do
        sed -i "s|^mirrorlist=|#mirrorlist=|; s|^# *baseurl=https://repo.almalinux.org/almalinux/\$releasever/|baseurl=https://vault.almalinux.org/$VERSION_ID/|" "$repo"
    done
    # The same runtime packages docker/appimage/Dockerfile builds with.
    dnf install -y -q --setopt=install_weak_deps=False \
        alsa-lib fontconfig libX11 libX11-xcb libxcb libXcursor libXi libxkbcommon libxkbcommon-x11 \
        libwayland-client libwayland-cursor libwayland-egl >/dev/null
fi
how=bare
[ -z "$system_libs" ] || how="with the system libraries"
echo "Checking $image on $(cat /etc/redhat-release 2>/dev/null || uname -sr), $(ldd --version | head -n 1), $how"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$image" "$work/picasso.AppImage"
chmod +x "$work/picasso.AppImage"
(cd "$work" && ./picasso.AppImage --appimage-extract >/dev/null)
cd "$work/squashfs-root"

# The bundled libraries AppRun puts on the path: those the system lacks.
installed=$({ /sbin/ldconfig -p || ldconfig -p; } 2>/dev/null)
bundled=
used=0
for dir in "$PWD"/usr/lib/picasso/*; do
    if ! grep -q "^[[:space:]]*${dir##*/} (libc6,$abi" <<<"$installed"; then
        bundled=$bundled${bundled:+:}$dir
        used=$((used + 1))
    fi
done
echo "AppRun would use $used of $(ls usr/lib/picasso | wc -l) bundled libraries"

failed=0
# -r also resolves each symbol, which finds a function an older system
# library lacks.
if LD_LIBRARY_PATH=$bundled ldd -r usr/bin/picasso 2>&1 | grep -E 'not found|undefined symbol'; then
    echo "usr/bin/picasso needs the libraries or symbols above" >&2
    failed=1
fi
for dir in ${bundled//:/ }; do
    for file in "$dir"/*; do
        if LD_LIBRARY_PATH=$bundled ldd "$file" 2>&1 | grep 'not found'; then
            echo "$file needs the libraries above" >&2
            failed=1
        fi
    done
done
if [ "$failed" = 0 ]; then
    echo "Picasso and the libraries it would use find everything they need"
fi
exit $failed

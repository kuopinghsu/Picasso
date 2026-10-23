#!/usr/bin/env bash
# Package the release binary as dist/picasso-$VERSION-linux-x64.AppImage.
#
# Picasso links to glibc and ALSA and opens its display, keyboard, and font
# libraries at runtime. This bundles those libraries, and everything they
# need. glibc, libgcc, and the OpenGL and Vulkan drivers have to match the
# system, so they stay out. build/AppRun puts a bundled library on the path
# only when the system does not already provide it.
#
# VERSION overrides the version in the file name. CI sets it so untagged
# builds include the commit. Requires Linux x86_64, curl, and the runtime
# libraries listed below (the same packages the AppImage job installs).
#
# The AppImage needs at least the glibc of the machine that builds it. CI
# builds on AlmaLinux 8 (glibc 2.28); a local build runs on systems as new as
# the one it was built on.

set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    echo "AppImage packages are built on Linux x86_64" >&2
    exit 1
fi

if [ -z "${VERSION:-}" ]; then
    VERSION=$(cargo pkgid)
    VERSION=${VERSION##*@}
    VERSION=${VERSION##*#}
fi

target_dir=${CARGO_TARGET_DIR:-target}
bin=$target_dir/release/picasso
if [ ! -x "$bin" ]; then
    echo "Missing $bin; run 'make release' first" >&2
    exit 1
fi

name="picasso-$VERSION-linux-x64"
work=$target_dir/appimage
app=$work/Picasso.AppDir
rm -rf "$app"
mkdir -p dist "$work"

install -Dm 755 "$bin" "$app/usr/bin/picasso"
strip "$app/usr/bin/picasso"
install -Dm 755 build/AppRun "$app/AppRun"
install -Dm 644 build/picasso.desktop "$app/picasso.desktop"
install -Dm 644 build/picasso.png "$app/picasso.png"
install -Dm 644 build/picasso.png "$app/usr/share/icons/hicolor/256x256/apps/picasso.png"
install -Dm 644 LICENSE "$app/usr/share/doc/picasso/LICENSE"
install -Dm 644 README.md "$app/usr/share/doc/picasso/README.md"
ln -s picasso.png "$app/.DirIcon"

# Libraries Picasso links to or opens at runtime (X11, Wayland, and keyboard
# maps through winit, fonts through fontconfig, sound through ALSA), with
# everything they need.
wanted="libasound.so.2 libfontconfig.so.1 libxcb.so.1 libX11.so.6 libX11-xcb.so.1 libXcursor.so.1 libXi.so.6
    libxkbcommon.so.0 libxkbcommon-x11.so.0 libwayland-client.so.0 libwayland-cursor.so.0 libwayland-egl.so.1"
system='^(ld-linux-x86-64|libc|libm|libdl|libpthread|librt|libresolv|libutil|libgcc_s|libGL|libEGL|libGLX|libGLdispatch|libOpenGL|libvulkan)\.so'

{
    ldd "$bin"
    for lib in $wanted; do
        # awk exits at the first match, so ldconfig gets SIGPIPE. `|| true` keeps
        # pipefail from aborting the script; a missing library is reported below.
        file=$( { /sbin/ldconfig -p || ldconfig -p; } 2>/dev/null | awk -v name="$lib" '$1 == name && /x86-64/ { print $NF; exit }' || true)
        if [ -z "$file" ]; then
            echo "${GITHUB_ACTIONS:+::error::}$lib is not installed" >&2
            exit 1
        fi
        echo "$lib $file"
        ldd "$file"
    done
} | awk '$2 ~ /^\// { print $1, $2 } $2 == "=>" && $3 ~ /^\// { print $1, $3 }' | sort -u |
while read -r lib file; do
    if [[ ! $lib =~ $system ]]; then
        install -Dm 644 "$(readlink -f "$file")" "$app/usr/lib/picasso/$lib/$lib"
    fi
done

echo "Bundled libraries:"
ls "$app/usr/lib/picasso"

tool=$work/appimagetool-x86_64.AppImage
if [ ! -x "$tool" ]; then
    curl -fsSL -o "$tool" \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x "$tool"
fi
# --appimage-extract works without FUSE. The extracted tool then packs the image.
rm -rf "$work/squashfs-root"
(cd "$work" && ./appimagetool-x86_64.AppImage --appimage-extract >/dev/null)
rm -f "dist/$name.AppImage"
ARCH=x86_64 VERSION="$VERSION" "$work/squashfs-root/AppRun" --no-appstream "$app" "dist/$name.AppImage"

echo "Created dist/$name.AppImage"

# What scripts/package-appimage.sh needs to build the Linux AppImage, as the
# AppImage job in .github/workflows/ci.yml builds it, so `make appimage-docker`
# needs only Docker. AlmaLinux 8 has glibc 2.28, the glibc of RHEL 8.0 to 8.10
# (8.4 included), so the AppImage runs there and on anything newer.
#
# The runtime libraries are installed without their -devel packages, as in
# CI, so the build does not record AlmaLinux's library folders and Picasso
# looks them up by name, as build/AppRun arranges.
FROM almalinux:8

COPY scripts/dnf-install.sh /tmp/dnf-install.sh
RUN bash /tmp/dnf-install.sh \
        gcc make pkgconf-pkg-config alsa-lib-devel git curl tar gzip jq file desktop-file-utils python3 \
        fontconfig libX11 libX11-xcb libxcb libXcursor libXi libxkbcommon libxkbcommon-x11 \
        libwayland-client libwayland-cursor libwayland-egl \
    && dnf clean all \
    && rm -rf /var/cache/dnf /tmp/dnf-install.sh

# Rust stable, readable by any user, since the build runs as the person who
# started it. CARGO_HOME is set again at run time to a folder the build can
# write (the registry cache); the tools stay here on PATH.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
    && chmod -R a+rX /usr/local/rustup /usr/local/cargo \
    && rustc --version

# build.rs asks git for the commit; the mounted checkout belongs to another user.
RUN git config --system --add safe.directory '*'

WORKDIR /work

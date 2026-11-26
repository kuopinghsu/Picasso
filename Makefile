CARGO   ?= cargo
PYTHON  ?= python3
# Where `make install` puts Picasso on Linux.
PREFIX  ?= $(HOME)/.local

# zsh, the macOS shell, passes `PREFIX=~/apps` on without expanding the `~`,
# and the recipes quote paths, so a leading `~/` is expanded here. Otherwise
# a folder named `~` would appear in the repository.
home = $(patsubst ~/%,$(HOME)/%,$(patsubst ~,$(HOME),$(1)))
DEST       = $(call home,$(PREFIX))
CORPUS_DIR = $(call home,$(CORPUS))

# `cargo pkgid` prints ".../picasso#0.9.3" (or "...#picasso@0.9.3").
# Make before 4.3 (macOS ships 3.81) treats a bare `#` inside a function call as a comment.
# Asked only by the targets that use it, as sudo often has no cargo on PATH.
HASH    := \#
VERSION  = $(lastword $(subst @, ,$(subst $(HASH), ,$(shell $(CARGO) pkgid))))

ifeq ($(OS),Windows_NT)
  PLATFORM := windows
else
  PLATFORM := $(shell uname -s | tr A-Z a-z)
endif

# On macOS, a build signed ad hoc is a new app to the Keychain each time,
# which asks again for each saved password, twice. A self-signed
# code-signing identity of this name, made once with `make sign-identity`,
# signs every build the same way, so "Always Allow" lasts (README.md).
# Without it, builds keep cargo's ad-hoc signature. The identity is
# self-signed, so `find-identity` is asked without -v, which lists trusted
# ones only.
SIGN_IDENTITY ?= Picasso Development
has_identity = security find-identity -p codesigning 2>/dev/null | grep -q '"$(SIGN_IDENTITY)"'
# A plain shell `if` here would need a POSIX shell to parse it, which this
# make.exe doesn't reliably pick on Windows (it runs recipes with cmd.exe,
# regardless of a Makefile SHELL override); deciding in Make itself instead
# of in the recipe's shell sidesteps that, and signing has nothing to do on
# Windows anyway.
ifeq ($(PLATFORM),darwin)
sign = if $(has_identity); then codesign --force --sign "$(SIGN_IDENTITY)" --identifier app.Picasso.picasso $(1); \
	else echo "Not signed with \"$(SIGN_IDENTITY)\": run make sign-identity once to keep Keychain approvals across builds"; fi
else
sign = true
endif

# `make` alone builds the release binary; `make help` lists every target.
.DEFAULT_GOAL := release

.PHONY: help release debug run run-stats version fmt fmt-check check lint test corpus bench bench-check check-docs deny ci translations \
	sign-identity notices release-notes guide guide-pdf user-guide-pdf theme appimage appimage-docker appimage-docker-check install uninstall world-map clean

# Each target's `##` text is what `make help` prints; `##@` starts a section.
# Printed by Python, as on Windows make runs recipes with cmd.exe, which has
# no awk and keeps echo's quotes.
help: ## List the targets and the variables they take
	@$(PYTHON) scripts/make-help.py Makefile $(VERSION)

##@ Build and run

# Release build. `make` and `make release` both produce target/release/picasso.
release: ## Build target/release/picasso (the default)
	$(CARGO) build --release
	@$(call sign,target/release/picasso)

# Once per Mac. codesign asks for the login keychain the first time it signs.
sign-identity: ## Make the self-signed macOS code-signing identity builds are signed with
	@if [ "$(PLATFORM)" != darwin ]; then echo "Only macOS signs builds"; \
	elif $(has_identity); then echo "\"$(SIGN_IDENTITY)\" is already in the keychain"; \
	else bash scripts/self-signed-identity.sh "$(SIGN_IDENTITY)"; fi

# Compiles faster than release, but scanning, thumbnails, and editing are much slower.
debug: ## Build target/debug/picasso, quicker to compile
	$(CARGO) build

# Launch the release binary. Extra arguments are photo folders:
#   make run ARGS="~/Pictures ~/Photos"
run: release ## Build and start Picasso; ARGS adds photo folders
	target/release/picasso $(ARGS)

# Prints frame times to the terminal every five seconds (docs/PERFORMANCE.md).
run-stats: ## Start Picasso printing frame times (PICASSO_FRAME_STATS=1)
	PICASSO_FRAME_STATS=1 $(CARGO) run --release -- $(ARGS)

version: ## Print the version in Cargo.toml
	@echo $(VERSION)

##@ Check

fmt: ## Format the Rust code
	$(CARGO) fmt

fmt-check: ## Check Rust formatting without changing files
	$(CARGO) fmt --check

check: ## Type-check the program and its tests without building
	$(CARGO) check --locked --all-targets

# Shows Clippy's suggestions; it doesn't fail on them.
lint: ## Run Clippy on the program and its tests
	$(CARGO) clippy --release --locked --all-targets

# Unit tests, as CI runs them. T limits them by name: make test T=backup
test: ## Run every test, as CI does; T=name runs only matching ones
	$(CARGO) test --release --locked $(T)

# Every photo and video in CORPUS through the reading checks (docs/TESTING.md).
corpus: ## Check your own photos: make corpus CORPUS=~/corpus
ifeq ($(strip $(CORPUS)),)
	@echo "Set CORPUS to a folder of photos: make corpus CORPUS=~/corpus" >&2
	@exit 1
else
	PICASSO_CORPUS='$(CORPUS_DIR)' $(CARGO) test --release --locked corpus -- --nocapture
endif

# Timings for a large library (see docs/PERFORMANCE.md). Sizes:
#   make bench PICASSO_BENCH_PHOTOS=500000 PICASSO_BENCH_FILES=10000
bench: ## Time a large made-up library (docs/PERFORMANCE.md)
	$(CARGO) test --release --locked bench_large_library -- --ignored --nocapture --test-threads=1

# A small run of the same timings that fails on a large regression, as CI runs it.
bench-check: ## A small benchmark that fails past its limits, as CI runs it
	PICASSO_BENCH_CHECK=1 PICASSO_BENCH_PHOTOS=20000 PICASSO_BENCH_FILES=500 $(MAKE) bench

# Documentation against the code: links, make targets, scripts, settings, and
# CHANGELOG.md sections (scripts/check-docs.py).
check-docs: ## Check the docs' links, make targets, scripts, and settings
	$(PYTHON) scripts/check-docs.py

# Security advisories, licenses, and sources of the dependencies (deny.toml).
# Needs cargo-deny: cargo install cargo-deny --locked
deny: ## Check the dependencies' advisories and licenses (needs cargo-deny)
	$(CARGO) deny --locked check

# What CI checks before it packages, so a failure shows up before a push.
ci: fmt-check check-docs deny test bench-check ## Run the checks CI runs: formatting, docs, deny, tests, bench-check (needs cargo-deny)

##@ Package

# THIRD-PARTY-NOTICES.txt for this platform, as every package carries it.
notices: ## Write THIRD-PARTY-NOTICES.txt for this platform
	$(PYTHON) scripts/third-party-notices.py -o THIRD-PARTY-NOTICES.txt

# The release notes CI publishes for the version in Cargo.toml, from CHANGELOG.md.
release-notes: ## Print the release notes for this version, from CHANGELOG.md
	bash scripts/release-notes.sh $(VERSION)

# The User Guide in both languages, each one HTML file with its pictures inside
# that opens offline. Needs Asciidoctor: brew install asciidoctor, or
# sudo apt install asciidoctor.
guide: ## Build the User Guide as HTML in target/guide (needs asciidoctor)
	asciidoctor --failure-level=WARN -a data-uri -a webfonts! -a relfilesuffix=.html -D target/guide \
		docs/USER_GUIDE.adoc docs/USER_GUIDE_zh-TW.adoc
	@echo "Open target/guide/USER_GUIDE.html or target/guide/USER_GUIDE_zh-TW.html"

# The same as PDF, as CI publishes it, with the themes in docs/ (a title page
# with the banner, the version and the month). scripts/docs-pdf.sh fetches
# Noto Sans TC once into target/guide-fonts. Needs asciidoctor-pdf, curl,
# Python 3 with fontTools and DejaVu Sans: on Debian and Ubuntu,
# gem install asciidoctor-pdf, then sudo apt install python3-fonttools
# fonts-dejavu-core. A character no font has fails the build.
guide-pdf: ## Build the User Guide as PDF in target/guide (needs asciidoctor-pdf)
	bash scripts/docs-pdf.sh --out target/guide docs/USER_GUIDE.adoc docs/USER_GUIDE_zh-TW.adoc

# The same PDFs with nothing installed but Docker: each docs/*.adoc becomes
# docs/*.pdf beside it, made in the image in docker/guide-pdf, which has what
# guide-pdf needs. The image is built the first time and reused after; the
# fonts are fetched once into target/guide-fonts. DOCS picks the files; files
# ending in _zh-TW take the Chinese theme.
DOCKER ?= docker
DOCS ?= $(wildcard docs/*.adoc)
GUIDE_IMAGE = picasso-guide-pdf:asciidoctor-pdf-2.3.15-noto
# On Windows make runs recipes with cmd.exe, or with Git Bash's sh when it is
# on PATH; Docker Desktop gives the files to the person, and
# MSYS_NO_PATHCONV, in the recipes' environment for either, keeps Git Bash
# from rewriting the container's paths. Elsewhere the files are made as the
# person rather than root. appimage-docker runs its container the same way.
ifeq ($(PLATFORM),windows)
DOCKER_RUN = $(DOCKER) run --rm
export MSYS_NO_PATHCONV := 1
else
DOCKER_RUN = $(DOCKER) run --rm --user "$$(id -u):$$(id -g)"
endif
user-guide-pdf: ## Convert docs/*.adoc to docs/*.pdf in Docker (needs only Docker)
	$(DOCKER) build --quiet -t $(GUIDE_IMAGE) docker/guide-pdf
	$(DOCKER_RUN) -v "$(CURDIR):/work" -w /work $(GUIDE_IMAGE) bash scripts/docs-pdf.sh $(DOCS)

# The optional WordPress theme in wordpress/voyagelight and the Picasso Places
# plugin in wordpress/picasso-places, as the ZIPs that Upload Theme and Upload
# Plugin take, an English and a Traditional Chinese one of each, in wordpress/. Fails if a text lacks a translation or the plugin's copy of a
# file shared with the theme differs from the theme's.
theme: ## Build the WordPress theme and plugin ZIPs, English and Chinese, in wordpress/
	$(PYTHON) scripts/build-theme.py --check
	$(PYTHON) scripts/build-theme.py

# After a text is added to or changed in the theme's or plugin's PHP: rewrites
# each .pot from the PHP and brings each .po in line, listing the texts left
# to translate. `make theme` fails until they are translated.
translations: ## Update the WordPress theme's and plugin's .pot and .po files from their PHP
	$(PYTHON) scripts/build-theme.py --translations

# Linux AppImage for this machine (x86_64 or aarch64) in dist/, the same AppDir
# as the CI AppImage job.
appimage: release ## Build the Linux AppImage in dist/
ifeq ($(PLATFORM),linux)
	VERSION='$(VERSION)' bash scripts/package-appimage.sh
else
	@echo AppImage packages are built on Linux >&2
	@exit 1
endif

# The same AppImage with nothing installed but Docker, on any system, built in
# the AlmaLinux 8 image in docker/appimage as CI builds it. AlmaLinux 8 has
# glibc 2.28, so the AppImage runs on RHEL 8.4 and every RHEL 8 or newer
# system, which one built by `make appimage` on a newer Linux does not.
# APPIMAGE_PLATFORM picks the processor: linux/amd64 (the default) or
# linux/arm64, emulated when it is not the machine's own. The build, Rust's
# package cache, and appimagetool stay in target/docker-linux-<arch> between
# runs; on Windows and macOS, where Docker Desktop's shared folders make a
# Rust build several times slower, they stay in a Docker volume of that name
# instead. The AppImage lands in dist/. VERSION=... on the command line
# renames it. RUST_VERSION is the toolchain in the image, part of its tag so
# a new one builds a new image; keep it at least Cargo.toml's rust-version
# (docs/RELEASING.md). The image's packages are cached with it, so
# APPIMAGE_PULL=1 fetches AlmaLinux 8 anew and reinstalls them, for the
# latest security fixes in the bundled libraries, as CI has every run. A
# build waits for another one on the same build folder to finish, since both
# would rebuild the same AppDir.
APPIMAGE_PLATFORM ?= linux/amd64
RUST_VERSION ?= 1.100.0
APPIMAGE_PULL ?=
APPIMAGE_ARCH = $(word 2,$(subst /, ,$(APPIMAGE_PLATFORM)))
APPIMAGE_IMAGE = picasso-appimage:almalinux8-rust$(RUST_VERSION)-$(APPIMAGE_ARCH)
APPIMAGE_TARGET = target/docker-linux-$(APPIMAGE_ARCH)
ifeq ($(PLATFORM),linux)
APPIMAGE_RUN = $(DOCKER_RUN)
else
APPIMAGE_RUN = $(DOCKER) run --rm -v picasso-docker-linux-$(APPIMAGE_ARCH):/work/$(APPIMAGE_TARGET)
endif
appimage-docker: ## Build the Linux AppImage for RHEL 8 and newer in Docker (needs only Docker)
	$(DOCKER) build --platform $(APPIMAGE_PLATFORM) $(if $(APPIMAGE_PULL),--pull --no-cache) \
		--build-arg RUST_VERSION=$(RUST_VERSION) -t $(APPIMAGE_IMAGE) -f docker/appimage/Dockerfile .
	$(APPIMAGE_RUN) --platform $(APPIMAGE_PLATFORM) -v "$(CURDIR):/work" -w /work \
		-e CARGO_TARGET_DIR=$(APPIMAGE_TARGET) -e CARGO_HOME=/work/$(APPIMAGE_TARGET)/cargo \
		$(if $(filter command line,$(origin VERSION)),-e "VERSION=$(VERSION)") \
		$(APPIMAGE_IMAGE) bash -c "mkdir -p $(APPIMAGE_TARGET) && flock $(APPIMAGE_TARGET)/.build.lock \
		bash -c 'cargo build --release --locked && bash scripts/package-appimage.sh'"

# The newest AppImage in dist/ for APPIMAGE_PLATFORM, unpacked in AlmaLinux
# 8.4 twice (scripts/check-appimage.sh): bare, where AppRun uses every
# bundled library, and with 8.4's own display, sound, and font libraries,
# which AppRun prefers, as on a desktop. Each time Picasso and the libraries
# it would use must find every library and symbol they need. The second
# fetches packages from vault.almalinux.org, as root in its container.
APPIMAGE_CHECK_IMAGE ?= almalinux:8.4
APPIMAGE_CHECK = $(DOCKER) run --rm --platform $(APPIMAGE_PLATFORM) -v "$(CURDIR):/work:ro" -w /work \
	$(APPIMAGE_CHECK_IMAGE) bash scripts/check-appimage.sh
appimage-docker-check: ## Check the AppImage in dist/ on RHEL 8.4, bare and with its libraries, in Docker
	$(APPIMAGE_CHECK) $(APPIMAGE_ARCH)
	$(APPIMAGE_CHECK) --system-libs $(APPIMAGE_ARCH)

# The program, menu entry, and icon under PREFIX, as the .tar.gz describes;
# PREFIX/bin must be on PATH for the menu entry to start it. Desktops that
# cache menu entries are told when update-desktop-database is there.
UPDATE_DESKTOP = command -v update-desktop-database >/dev/null && update-desktop-database '$(DEST)/share/applications' || true

# As root, as under sudo, the default PREFIX would be root's home, not the
# user's, so PREFIX must be given. Root doesn't build either, which would
# leave root's files in target/: it installs what `make` built.
ROOT_PREFIX = if [ "$$(id -u)" = 0 ] && [ '$(origin PREFIX)' = file ]; then \
	echo "As root, give PREFIX, such as: sudo make $@ PREFIX=/usr/local" >&2; exit 1; fi

install: ## Install for this user on Linux (PREFIX, default ~/.local)
ifeq ($(PLATFORM),linux)
	@$(ROOT_PREFIX)
	@if [ "$$(id -u)" != 0 ]; then $(CARGO) build --release; \
	elif [ ! -x target/release/picasso ]; then echo "Run make as yourself first; then sudo make install copies it" >&2; exit 1; fi
	install -Dm 755 target/release/picasso '$(DEST)/bin/picasso'
	install -Dm 644 build/picasso.desktop '$(DEST)/share/applications/picasso.desktop'
	install -Dm 644 build/picasso.png '$(DEST)/share/icons/hicolor/256x256/apps/picasso.png'
	@$(UPDATE_DESKTOP)
	@echo "Installed in $(DEST); make uninstall removes it and leaves your library and settings."
else
	@echo "make install is for Linux; use the .exe, or Picasso.app from the .dmg" >&2
	@exit 1
endif

uninstall: ## Remove what make install put under PREFIX
ifeq ($(PLATFORM),linux)
	@$(ROOT_PREFIX)
	rm -f '$(DEST)/bin/picasso' '$(DEST)/share/applications/picasso.desktop' \
		'$(DEST)/share/icons/hicolor/256x256/apps/picasso.png'
	@$(UPDATE_DESKTOP)
else
	@echo "make uninstall is for Linux" >&2
	@exit 1
endif

##@ Maintain

# build/world.bin, the Travel Map's borders, from Natural Earth's public-domain
# data, in a virtual environment of its own (scripts/world-map.py). The data
# is downloaded into target/natural-earth the first time. On macOS and Linux.
world-map: ## Rebuild build/world.bin from Natural Earth data
	$(PYTHON) -m venv target/world-map-venv
	target/world-map-venv/bin/pip install --quiet mapbox-earcut numpy
	target/world-map-venv/bin/python scripts/world-map.py target/natural-earth

clean: ## Delete build output, dist/, and THIRD-PARTY-NOTICES.txt
	$(CARGO) clean
	rm -rf dist THIRD-PARTY-NOTICES.txt

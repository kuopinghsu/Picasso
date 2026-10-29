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
# which asks again for each saved password, twice. A code-signing identity
# of this name, made once in Keychain Access, signs every build the same
# way, so "Always Allow" lasts (README.md). Without it nothing changes.
SIGN_IDENTITY ?= Picasso Development
sign = if [ "$(PLATFORM)" = darwin ] && security find-identity -v -p codesigning 2>/dev/null | grep -q '"$(SIGN_IDENTITY)"'; then \
	codesign --force --sign "$(SIGN_IDENTITY)" --identifier app.Picasso.picasso $(1); fi

# `make` alone builds the release binary; `make help` lists every target.
.DEFAULT_GOAL := release

.PHONY: help release debug run run-stats version check lint test corpus bench bench-check check-docs deny ci \
	notices release-notes guide guide-pdf appimage install uninstall world-map clean

# Each target's `##` text is what `make help` prints; `##@` starts a section.
help: ## List the targets and the variables they take
	@echo "Picasso $(VERSION). Usage: make [target] [VARIABLE=value]"
	@awk 'BEGIN { FS = ":.*## " } \
		/^##@ / { printf "\n%s\n", substr($$0, 5) } \
		/^[a-z][a-z0-9-]*:.*## / { printf "  %-14s %s\n", $$1, $$2 }' $(MAKEFILE_LIST)
	@echo
	@echo "Variables"
	@echo "  ARGS=...       photo folders for run and run-stats: make run ARGS=\"~/Pictures\""
	@echo "  T=...          only the tests whose name contains it: make test T=backup"
	@echo "  CORPUS=dir     folder of your own photos for make corpus"
	@echo "  PREFIX=dir     where install puts Picasso (default ~/.local)"
	@echo "  SIGN_IDENTITY  macOS code-signing identity for builds (default Picasso Development)"
	@echo "  PICASSO_BENCH_PHOTOS, PICASSO_BENCH_FILES   sizes for make bench (docs/PERFORMANCE.md)"

##@ Build and run

# Release build. `make` and `make release` both produce target/release/picasso.
release: ## Build target/release/picasso (the default)
	$(CARGO) build --release
	@$(call sign,target/release/picasso)

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
ci: check-docs deny test bench-check ## Run the checks CI runs: docs, deny, tests, bench-check (needs cargo-deny)

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

# The same as PDF, as CI publishes it. The themes in docs/ take the Chinese and
# symbols from Droid Sans Fallback and DejaVu Sans, looked for in GUIDE_FONTS:
# on Debian and Ubuntu, gem install asciidoctor-pdf, then
# sudo apt install fonts-droid-fallback fonts-dejavu-core. A character no font
# has fails the build.
GUIDE_FONTS ?= /usr/share/fonts/truetype/droid;/usr/share/fonts/truetype/dejavu
# --verbose, because asciidoctor-pdf reports a missing character only then.
GUIDE_PDF = asciidoctor-pdf --verbose --failure-level=WARN -a pdf-themesdir=docs \
	-a 'pdf-fontsdir=$(GUIDE_FONTS);GEM_FONTS_DIR' -D target/guide

guide-pdf: ## Build the User Guide as PDF in target/guide (needs asciidoctor-pdf)
	$(GUIDE_PDF) -a pdf-theme=guide docs/USER_GUIDE.adoc
	$(GUIDE_PDF) -a pdf-theme=guide-zh-TW docs/USER_GUIDE_zh-TW.adoc

# Linux AppImage for this machine (x86_64 or aarch64) in dist/, the same AppDir
# as the CI AppImage job.
appimage: release ## Build the Linux AppImage in dist/
ifeq ($(PLATFORM),linux)
	VERSION='$(VERSION)' bash scripts/package-appimage.sh
else
	@echo AppImage packages are built on Linux >&2
	@exit 1
endif

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

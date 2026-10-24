CARGO ?= cargo

# `cargo pkgid` prints ".../picasso#0.9.3" (or "...#picasso@0.9.3").
# Make before 4.3 (macOS ships 3.81) treats a bare `#` inside a function call as a comment.
HASH    := \#
VERSION := $(lastword $(subst @, ,$(subst $(HASH), ,$(shell $(CARGO) pkgid))))

ifeq ($(OS),Windows_NT)
  PLATFORM := windows
else
  PLATFORM := $(shell uname -s | tr A-Z a-z)
endif

.PHONY: release run appimage clean

# Release build. `make` and `make release` both produce target/release/picasso.
release:
	$(CARGO) build --release

# Launch the release binary. Extra arguments are photo folders:
#   make run ARGS="~/Pictures ~/Photos"
run:
	$(CARGO) run --release -- $(ARGS)

# Linux x86_64 AppImage in dist/, the same AppDir as the CI AppImage job.
appimage: release
ifeq ($(PLATFORM),linux)
	VERSION='$(VERSION)' bash scripts/package-appimage.sh
else
	@echo AppImage packages are built on Linux x86_64 >&2
	@exit 1
endif

clean:
	$(CARGO) clean
	rm -rf dist

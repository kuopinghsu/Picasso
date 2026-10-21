.PHONY: release run clean

# Release build. `make` and `make release` both produce target/release/picasso.
release:
	cargo build --release

# Launch the release binary. Extra arguments are photo folders:
#   make run ARGS="~/Pictures ~/Photos"
run:
	cargo run --release -- $(ARGS)

clean:
	cargo clean

.PHONY: install build test lint fmt check

install:
	cargo install --path crates/cli

build:
	cargo build --release

test:
	cargo test --workspace

lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

check: fmt lint test
	@echo "All checks passed"

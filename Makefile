# ──────────────────────────────────────────────────────────────────────
# Makefile — Standardized Build, Test, and Release Targets
# ──────────────────────────────────────────────────────────────────────

.PHONY: all build check test run chat chat-release clean py-build fmt clippy release

# Default target
all: check test build

# Compile all workspace crates in debug mode
build:
	cargo build --workspace

# Compile all workspace crates with maximum release optimizations
release:
	cargo build --release --workspace

# Fast syntax and type checking across all workspace members
check:
	cargo check --workspace

# Execute the entire unit and integration test suite
test:
	cargo test --workspace -- --nocapture

# Launch the interactive terminal UI & Groq AI chat session
run:
	cargo run --bin harness

# Launch interactive Groq AI autonomous chat session
chat:
	cargo run --bin harness

# Launch production-optimized Groq AI autonomous chat session
chat-release:
	cargo run --release --bin harness

# Build the native PyO3 Python extension module via maturin
py-build:
	maturin develop --manifest-path crates/harness-bridge/Cargo.toml --release

# Format all Rust source files
fmt:
	cargo fmt --all

# Run Clippy linter with strict warning enforcement
clippy:
	cargo clippy --workspace --all-targets -- -D warnings

# Remove build artifacts and temporary files
clean:
	cargo clean
	rm -rf .harness/exports/*

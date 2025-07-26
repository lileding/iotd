.PHONY: check fmt clippy test build clean install-hooks

# Run all checks (fmt, clippy, test)
check: fmt-check clippy test

# Check code formatting
fmt-check:
	cargo fmt -- --check

# Format code
fmt:
	cargo fmt

# Run clippy lints
clippy:
	cargo clippy -- -D warnings

# Run tests
test:
	cargo test

# Build release binary
build:
	cargo build --release

# Clean build artifacts
clean:
	cargo clean

# Install git hooks
install-hooks:
	./scripts/install-hooks.sh

# Run checks and fix issues
fix: fmt
	@echo "Code formatted. Please review clippy warnings manually."
	cargo clippy
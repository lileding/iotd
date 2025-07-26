#!/bin/bash
#
# Install git hooks for the project

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOKS_DIR="$PROJECT_ROOT/.git/hooks"

echo "Installing git hooks..."

# Create hooks directory if it doesn't exist
mkdir -p "$HOOKS_DIR"

# Create pre-commit hook
cat > "$HOOKS_DIR/pre-commit" << 'EOF'
#!/bin/bash
#
# Pre-commit hook that runs cargo fmt and cargo clippy checks
# before allowing a commit to proceed.

set -e

echo "Running pre-commit checks..."

# Check if we're in a Rust project
if [ ! -f "Cargo.toml" ]; then
    echo "Not in a Rust project root, skipping checks"
    exit 0
fi

# Run cargo fmt check
echo "Checking code formatting..."
if ! cargo fmt -- --check; then
    echo ""
    echo "❌ Code formatting check failed!"
    echo "Please run 'cargo fmt' to fix formatting issues."
    exit 1
fi
echo "✅ Code formatting check passed"

# Run cargo clippy
echo ""
echo "Running clippy lints..."
if ! cargo clippy -- -D warnings; then
    echo ""
    echo "❌ Clippy check failed!"
    echo "Please fix the clippy warnings before committing."
    exit 1
fi
echo "✅ Clippy check passed"

echo ""
echo "All pre-commit checks passed! ✨"
EOF

# Make the hook executable
chmod +x "$HOOKS_DIR/pre-commit"

echo "✅ Git hooks installed successfully!"
echo ""
echo "The following hooks have been installed:"
echo "  - pre-commit: Runs 'cargo fmt --check' and 'cargo clippy'"
echo ""
echo "To bypass hooks temporarily, use: git commit --no-verify"
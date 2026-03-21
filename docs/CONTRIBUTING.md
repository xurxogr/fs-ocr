# Contributing to fs-ocr

## Development Setup

### Prerequisites

- Rust toolchain (1.70+)
- Python 3.10+
- System libraries (Ubuntu/Debian):

```bash
sudo apt-get install \
    libhdf5-dev \
    libtesseract-dev \
    libleptonica-dev \
    libclang-dev
```

### Install for Development

```bash
# Clone the repository
git clone <repo-url>
cd fs-ocr

# Install Python dev dependencies
pip install maturin pytest numpy

# Build and install the Python module
maturin develop
```

## Available Commands

<!-- AUTO-GENERATED from Cargo.toml -->
| Command | Description |
|---------|-------------|
| `cargo test` | Run all 46 unit tests |
| `cargo clippy` | Run Clippy linter |
| `cargo fmt` | Format code with rustfmt |
| `cargo build` | Build debug version |
| `cargo build --release` | Build optimized release |
| `maturin develop` | Build + install Python module |
| `maturin build --release` | Build distributable wheel |
<!-- END AUTO-GENERATED -->

## Testing

### Run All Tests

```bash
cargo test
```

### Run Specific Test

```bash
cargo test test_name
cargo test template::  # All template module tests
```

### Test with Output

```bash
cargo test -- --nocapture
```

## Code Style

- **Formatter**: `cargo fmt` before committing
- **Linter**: `cargo clippy -- -D warnings` (treat warnings as errors)
- **Max line width**: 100 characters (rustfmt default)
- **Naming**: `snake_case` for functions, `PascalCase` for types

## Pull Request Checklist

- [ ] All tests pass (`cargo test`)
- [ ] No Clippy warnings (`cargo clippy`)
- [ ] Code formatted (`cargo fmt`)
- [ ] New features have tests
- [ ] Updated docs if API changed

## Architecture Overview

See [docs/CODEMAPS/architecture.md](CODEMAPS/architecture.md) for module structure.

## Common Tasks

### Add a New Enum Variant

1. Add variant to `src/enums/<enum_name>.rs`
2. Update `From<u8>` impl
3. Update `from_string()` if applicable
4. Run tests

### Modify Template Matching

1. Edit `src/template/matching.rs`
2. Run `cargo test template::` to verify
3. Check performance impact with large databases

### Add Python API Method

1. Add method to `src/lib.rs` with `#[pymethods]`
2. Add `#[pyo3(signature = (...))]` for optional args
3. Document with docstring
4. Update README API section

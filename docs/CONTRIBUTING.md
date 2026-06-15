# Contributing to fs-ocr

## Development Setup

### Prerequisites

- Rust toolchain (edition 2021)
- Python 3.10+
- Build tools (Ubuntu/Debian) — HDF5 is built from source via `static-hdf5`:

```bash
sudo apt-get install cmake gcc g++ libclang-dev
```

OCR is pure-Rust (`ocrs`/`rten`) with the recognition model embedded in the
binary — no Tesseract, Leptonica, or OpenCV needed to build or run. The only
optional piece is the **system `tesseract` CLI**, used at runtime to read
Chinese custom names if it happens to be installed (no build flag involved):

```bash
sudo apt-get install tesseract-ocr   # optional, runtime-only, Chinese names
```

### Install for Development

```bash
# Clone the repository
git clone <repo-url>
cd fs-ocr

# Install Python dev dependencies
pip install maturin pytest numpy

# Build and install the Python module (default features include `python`)
maturin develop --release
```

## Available Commands

<!-- AUTO-GENERATED from Cargo.toml -->
| Command | Description |
|---------|-------------|
| `cargo test` | Run the unit test suite |
| `cargo clippy --all-targets -- -D warnings` | Run Clippy linter (warnings as errors) |
| `cargo fmt` | Format code with rustfmt |
| `cargo build` | Build debug version |
| `cargo build --release` | Build optimized release |
| `cargo build --release --no-default-features --bin fs-ocr` | Build the standalone CLI (no libpython linkage) |
| `maturin develop --release` | Build + install Python module |
| `maturin build --release` | Build distributable wheel |
| `maturin build --release --features static-hdf5,pyo3/extension-module` | CI-style abi3 wheel with static HDF5 |
<!-- END AUTO-GENERATED -->

## Feature Flags

<!-- AUTO-GENERATED from Cargo.toml [features] -->
| Feature | Default | Description |
|---------|---------|-------------|
| `python` | **on** | PyO3 + numpy bindings. Drop with `--no-default-features` for a pure CLI build with no libpython linkage. |
| `static-hdf5` | off | Build/statically link libhdf5 + zlib from source (used by CI wheels; needs CMake + a C/C++ compiler). |
<!-- END AUTO-GENERATED -->

## Environment Variables

<!-- AUTO-GENERATED from source (std::env::var) -->
| Variable | Required | Description |
|----------|----------|-------------|
| `FS_OCR_TIMING` | No | Set to `1` to include per-stage timing (ms) in the CLI's JSON output. |
| `FS_DEBUG_OCR` | No | Set to `1` to dump `debug_image.png` + per-region OCR crops to the cwd (debugging the OCR pipeline). |
| `FS_OCR_TESSERACT` | No | Path/name of the `tesseract` binary for Chinese custom names (default `tesseract`). |
| `FS_OCR_TESSERACT_LANG` | No | Tesseract language for Chinese custom names (default `chi_sim`). |
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

### Pre-commit Hooks

Formatting is enforced via pre-commit (`cargo fmt` for Rust, `ruff format` for
Python). If a hook reformats a file the commit aborts so you can re-stage.
Set up once per clone:

```bash
pip install pre-commit
pre-commit install
```

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

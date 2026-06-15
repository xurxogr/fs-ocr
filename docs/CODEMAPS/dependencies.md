<!-- Generated: 2026-06-15 | Files scanned: 41 | Token estimate: ~400 -->

# fs-ocr Dependencies

## Runtime Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| pyo3 | 0.24 | Python bindings (abi3-py310, optional via `python`) |
| numpy | 0.24 | NumPy array interop (optional via `python`) |
| ndarray | 0.16 | N-dimensional arrays |
| serde / serde_json | 1.0 | Serialization + JSON output |
| thiserror | 2.0 | Error derive macros |
| rayon | 1.10 | Parallel NCC matching |
| chrono | 0.4 | Timestamps (serde feature) |
| image | 0.25 | Image decoding (png/jpeg/bmp/gif/webp/tiff; defaults off) |
| clap | 4 | CLI argument parsing (derive) |
| hdf5 / hdf5-sys | 0.8 | HDF5 template database |
| ocrs | 0.11 | Pure-Rust OCR engine |
| rten / rten-imageproc | 0.22 | ML runtime + image ops for ocrs |

No Tesseract *crate* dependency. Chinese custom names are read via the
**system `tesseract` CLI** (optional, detected at runtime) — see tesseract.rs.

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| tempfile | 3.14 | Temporary files in tests |

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `python` | **on** | PyO3 + numpy bindings (the wheel build). Drop with `--no-default-features` for a pure CLI with no libpython linkage. |
| `static-hdf5` | off | Build/statically link libhdf5 + zlib from source (CI wheels). Needs CMake + C/C++. |

## Native Libraries (System)

| Library | Required By | Notes |
|---------|-------------|-------|
| libhdf5 | hdf5 crate | `apt install libhdf5-dev` (or `static-hdf5` to bundle) |
| CMake + C/C++ | `static-hdf5` | Build-time only |
| tesseract | Chinese names | Optional runtime CLI; feature degrades if absent |

Default build needs **no external OCR engine** — ocrs is pure Rust and the
recognition model is embedded.

## Embedded vs External Data

```
Embedded in the binary (include_bytes!, ship in wheel + CLI):
  data/text-recognition.rten   # ocrs recognition model (~10MB)
  data/type_templates.bin      # stockpile-type templates
  data/public_templates.bin    # "Public" default-name templates

External (NOT bundled — user supplies):
  data/fs_airborne.h5          # icon template DB (--database / database_path)
```

## Dependency Graph (simplified)

```
fs_ocr
├── pyo3 + numpy → Python interface (feature `python`)
├── clap → CLI binary (--no-default-features)
├── hdf5 → template loading
│   └── libhdf5 (native, or static-hdf5)
├── ocrs + rten → pure-Rust OCR (embedded model)
├── rayon → parallel matching
├── image → file/stdin decoding
└── serde_json → JSON output
(runtime, optional: system `tesseract` CLI for Chinese names)
```

## Build

```bash
# Python module (default features include `python`)
maturin develop --release

# CLI binary (no python / libpython linkage)
cargo build --release --no-default-features --bin fs-ocr

# CI wheels (static HDF5, abi3 extension-module)
maturin build --release --features static-hdf5,pyo3/extension-module

# Release profile: opt-level=3, lto=true, codegen-units=1
```

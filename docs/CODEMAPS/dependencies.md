<!-- Generated: 2026-03-21 | Files scanned: 24 | Token estimate: ~300 -->

# fs-ocr Dependencies

## Runtime Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| pyo3 | 0.22 | Python bindings (extension-module) |
| numpy | 0.22 | NumPy array interop |
| ndarray | 0.16 | N-dimensional arrays |
| serde | 1.0 | Serialization traits |
| serde_json | 1.0 | JSON serialization |
| thiserror | 2.0 | Error derive macros |
| rayon | 1.10 | Data parallelism |
| chrono | 0.4 | Timestamps (serde feature) |
| image | 0.25 | Image file loading |
| hdf5 | 0.8 | HDF5 template database |
| leptess | 0.14 | Tesseract OCR bindings |

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| tempfile | 3.14 | Temporary files in tests |

## Native Libraries (System)

| Library | Required By | Notes |
|---------|-------------|-------|
| libhdf5 | hdf5 crate | `apt install libhdf5-dev` |
| leptonica | leptess | Image processing for Tesseract |
| tesseract | leptess | OCR engine (v4+) |

## Python Environment

```bash
# Install Rust library as Python module
maturin develop --release

# Or build wheel
maturin build --release
```

## Dependency Graph (simplified)

```
fs_ocr
├── pyo3 + numpy → Python interface
├── hdf5 → Template loading
│   └── libhdf5 (native)
├── leptess → OCR
│   └── tesseract + leptonica (native)
├── rayon → Parallel matching
├── image → File loading
└── serde_json → JSON output
```

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `full` | off | Reserved for future native deps |

## Build Profile

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

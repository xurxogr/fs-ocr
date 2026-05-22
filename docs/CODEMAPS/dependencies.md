<!-- Generated: 2026-05-21 | Files scanned: 31 | Token estimate: ~400 -->

# fs-ocr Dependencies

## Runtime Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| pyo3 | 0.24 | Python bindings (extension-module) |
| numpy | 0.24 | NumPy array interop |
| ndarray | 0.16 | N-dimensional arrays |
| serde / serde_json | 1.0 | Serialization + JSON output |
| thiserror | 2.0 | Error derive macros |
| rayon | 1.10 | Parallel NCC matching |
| chrono | 0.4 | Timestamps (serde feature) |
| image | 0.25 | Image file / stdin decoding |
| clap | 4 | CLI argument parsing (derive) |
| base64 | 0.22 | Encoding helpers |
| hdf5 / hdf5-sys | 0.8 | HDF5 template database |
| ocrs | 0.11 | Pure-Rust OCR engine |
| rten / rten-imageproc | 0.22 | ML runtime + image ops for ocrs |
| leptess | 0.14 | Tesseract bindings (optional, `ocr-full`) |

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| tempfile | 3.14 | Temporary files in tests |

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `ocr-full` | off | Enable Tesseract backend via `leptess` (needs system Tesseract) |
| `static-hdf5` | off | Build/statically link libhdf5 + zlib from source (CI wheels) |

## Native Libraries (System)

| Library | Required By | Notes |
|---------|-------------|-------|
| libhdf5 | hdf5 crate | `apt install libhdf5-dev` (or `static-hdf5` to bundle) |
| CMake + C/C++ | `static-hdf5` | Build-time only |
| tesseract + leptonica | `ocr-full` | Only when Tesseract backend enabled |

Default build needs **no external OCR engine** — ocrs is pure Rust.

## OCR Model Files (data/)

```
data/
├── text-detection.rten      # ocrs detection model
├── text-recognition.onnx    # ocrs recognition model
└── renner_numbers.traineddata  # (Tesseract digit model, ocr-full path)
```

## Dependency Graph (simplified)

```
fs_ocr
├── pyo3 + numpy → Python interface
├── clap → CLI binary
├── hdf5 → template loading
│   └── libhdf5 (native, or static-hdf5)
├── ocrs + rten → pure-Rust OCR (default)
├── leptess → Tesseract OCR (optional)
│   └── tesseract + leptonica (native)
├── rayon → parallel matching
├── image → file/stdin decoding
└── serde_json → JSON output
```

## Build

```bash
# Python module (default, ocrs backend)
maturin develop --release

# With Tesseract backend
maturin build --release --features ocr-full

# CLI binary
cargo build --release --bin fs-ocr

# Release profile: opt-level=3, lto=true, codegen-units=1
```

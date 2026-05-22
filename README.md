# fs-ocr

Fast OCR library for Foxhole stockpile screenshots, written in Rust with Python bindings.

## Features

- **Fast Template Matching**: pHash filtering + NCC scoring with adaptive candidate escalation
- **ROI + Grey Mask Detection**: black-box ROI localization followed by grey-mask box detection
- **Quantity Recognition**: template-based glyph matching (no OCR engine needed for digits)
- **Pure-Rust OCR by default**: `ocrs`/`rten` for Latin text; optional Tesseract backend for Chinese/Russian (see [Chinese / Russian support](#chinese--russian-non-latin-support))
- **Python API + CLI**: PyO3 bindings (`import fs_ocr`) and an `fs-ocr` command-line tool

## Installation

### Standalone CLI (no Python required)

Prebuilt `fs-ocr` binaries are attached to each [GitHub Release](../../releases)
for Linux, Windows, and macOS (x86_64 + Apple Silicon). Download, extract, run:

```bash
tar -xzf fs-ocr-linux-x86_64.tar.gz   # or unzip the .zip on Windows
./fs-ocr scan screenshot.png -d templates.h5 --faction wardens
```

Two builds are published per platform:

- `fs-ocr-<os>-<arch>` — pure-Rust OCR, no system dependencies
- `fs-ocr-tesseract-<os>-<arch>` — Tesseract backend (requires system Tesseract)

### From PyPI

Two distributions are published from this codebase; install exactly one:

```bash
pip install fs-ocr            # pure-Rust OCR backend, zero system OCR libs
pip install fs-ocr-tesseract  # Tesseract backend (requires system Tesseract)
```

Both import as `import fs_ocr`.

> **Install exactly one.** The two distributions ship the same `fs_ocr`
> module and cannot coexist. pip will not stop you from installing both, so
> `import fs_ocr` raises a clear error if it detects both. To switch backends,
> uninstall first:
>
> ```bash
> pip uninstall -y fs-ocr fs-ocr-tesseract
> pip install fs-ocr           # or fs-ocr-tesseract
> ```

### Chinese / Russian (non-Latin) support

The default pure-Rust backend (`ocrs`) only recognizes **Latin** text. Screenshots
from the Chinese or Russian game clients need the Tesseract backend, which reads
those scripts. There is **no language flag** — you just install the right package
plus the matching language data, and scanning picks it up automatically (it loads
`eng+chi_sim+rus` when available and falls back to `eng`).

1. **Install the Tesseract distribution** instead of the default:

   ```bash
   pip install fs-ocr-tesseract
   ```

2. **Install system Tesseract + the language data.** The engine and traineddata
   are *not* bundled — they come from your OS package manager, which also keeps
   them patched for security. Tesseract finds them via the system `tessdata`
   directory (or `TESSDATA_PREFIX`):

   | OS | Command (engine + Simplified Chinese + Russian) |
   |----|--------------------------------------------------|
   | Debian/Ubuntu | `sudo apt install tesseract-ocr tesseract-ocr-chi-sim tesseract-ocr-rus` |
   | Fedora/RHEL | `sudo dnf install tesseract tesseract-langpack-chi_sim tesseract-langpack-rus` |
   | macOS (Homebrew) | `brew install tesseract tesseract-lang` (installs all languages) |
   | Windows | [UB Mannheim installer](https://github.com/UB-Mannheim/tesseract/wiki) — select Chinese/Russian during setup — or `choco install tesseract` |

   For Traditional Chinese, add `chi_tra` (e.g. `tesseract-ocr-chi-tra`).

3. **Scan as usual** — no code change. If a language is missing, recognition
   silently falls back to `eng`; install the package above to enable it.

> Prefer the smallest traineddata? You can drop `chi_sim.traineddata` /
> `rus.traineddata` from [`tessdata_fast`](https://github.com/tesseract-ocr/tessdata_fast)
> into your `tessdata` directory and point `TESSDATA_PREFIX` at it instead of
> using the OS packages.

### From Source

The default build needs only a C/C++ toolchain (HDF5 is built from source via
the `static-hdf5` feature). No OpenCV or Tesseract required for the default
backend:

```bash
# Build deps (Ubuntu/Debian)
sudo apt-get install cmake gcc g++ libclang-dev

# Build and install (default, pure-Rust OCR)
pip install maturin
maturin develop --release

# Optional: Tesseract backend (also needs system Tesseract/Leptonica)
sudo apt-get install libtesseract-dev libleptonica-dev
maturin develop --release --features ocr-full
```

## Python Usage

```python
from fs_ocr import StockpileScanner, ScanConfig
import numpy as np

# Create scanner (data_path holds the OCR model files, default "data")
scanner = StockpileScanner(database_path="templates.h5", data_path="data")

# Scan from NumPy array (H x W x 3, uint8, BGR)
image = np.array(...)  # Your image data
result = scanner.scan(image, faction="wardens")
print(result.to_json())

# Scan from file
result = scanner.scan_file("screenshot.png", faction="colonials")

# With custom config
config = ScanConfig(confidence_gap=0.02)
result = scanner.scan(image, config=config)

# Access result data
for item in result.items:
    print(f"{item.code}: {item.quantity} (confidence: {item.confidence:.2f})")
```

## API Reference

### StockpileScanner

Main scanner class.

```python
scanner = StockpileScanner(
    database_path: str,      # Path to HDF5 template database
    data_path: str = "data"  # Path to OCR model files directory
)

result = scanner.scan(
    image: np.ndarray,       # BGR image (H x W x 3, uint8)
    faction: str = None,     # "wardens", "colonials", or None
    config: ScanConfig = None
)

result = scanner.scan_file(
    image_path: str,         # Path to image file
    faction: str = None,
    config: ScanConfig = None
)
```

### ScanConfig

<!-- AUTO-GENERATED from src/config.rs -->
Configuration options for tuning the matching pipeline.

```python
config = ScanConfig(
    phash_threshold: int = 15,            # Max Hamming distance for pHash filter (lower=faster)
    max_ncc_candidates: int = 100,        # Hard cap on NCC candidates (upper bound of escalation)
    ncc_initial_candidates: int = 25,     # Initial NCC batch before adaptive escalation
    ncc_escalation_threshold: float = 0.9,# Escalate candidate count if best confidence below this
    confidence_gap: float = 0.0,          # Return alternatives within this gap of best match
    ncc_tiebreaker_threshold: float = 0.003  # Edge(Sobel)-based tiebreaker; 0.0 disables
)

# Serialize/deserialize
config.to_json() -> str
ScanConfig.from_json(json_str) -> ScanConfig
```
<!-- END AUTO-GENERATED -->

### Stockpile

Scan result containing detected items.

```python
stockpile.name             # Custom stockpile name (if applicable)
stockpile.type             # StockpileType enum
stockpile.is_reserve       # True when named something other than "Public"
stockpile.items            # List[StockpileItem]
stockpile.timestamp        # ISO 8601 scan timestamp
stockpile.shard            # Game shard name
stockpile.ingame_timestamp # In-game time (e.g. "Day 1293, 1906 Hours")
stockpile.resolution       # Screenshot resolution ("WxH")
stockpile.errors           # List of error messages
stockpile.timing           # Optional[Timing] per-stage metrics (None unless collected)
stockpile.to_json()        # Serialize to JSON (to_json_compact() for one line)
```

### StockpileItem

Individual detected item.

```python
item.code        # Item code or "Unknown"
item.quantity    # Detected quantity (-1 if failed)
item.crated      # Whether item is crated
item.confidence  # Match confidence (0.0 - 1.0)
item.candidates  # Alternative matches (if confidence_gap > 0)
```

## CLI Usage

The crate also builds an `fs-ocr` binary that emits JSON.

```bash
# Scan a file
fs-ocr scan screenshot.png -d templates.h5 --faction wardens

# Read image from stdin ("-" or omit the path)
cat screenshot.png | fs-ocr scan -d templates.h5 --compact

# Print version
fs-ocr version
```

Matching can be tuned with `--phash-threshold`, `--max-ncc-candidates`,
`--ncc-initial-candidates`, `--ncc-escalation-threshold`, `--ncc-tiebreaker`,
and `--confidence-gap`. Set `FS_OCR_TIMING=1` to include per-stage timing in the
output. Exit codes: `0` ok, `1` runtime error, `2` bad input.

## Building the Template Database

The template database is built from game assets. See the main foxhole-stockpiles repository for details on generating the HDF5 database.

## Development

<!-- AUTO-GENERATED from Cargo.toml + pyproject.toml -->
### Commands

| Command | Description |
|---------|-------------|
| `cargo test` | Run the Rust test suite |
| `cargo clippy -- -D warnings` | Run linter (warnings as errors) |
| `cargo fmt` | Format code with rustfmt |
| `cargo build --release` | Build optimized library |
| `cargo build --release --bin fs-ocr` | Build the CLI binary |
| `maturin develop --release` | Build and install Python module (dev) |
| `maturin build --release` | Build Python wheel for distribution |

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `ocr-full` | off | Tesseract backend via `leptess` (needs system Tesseract) |
| `static-hdf5` | off | Build/statically link libhdf5 from source (used by CI wheels) |

### Requirements

- Rust toolchain (edition 2021)
- Python 3.10+ (for bindings)
- Build tools: `cmake`, `gcc`/`g++`, `libclang-dev`
- Only for `ocr-full`: `libtesseract-dev`, `libleptonica-dev`

### Dev Dependencies

```bash
pip install pytest numpy  # Python dev deps
```
<!-- END AUTO-GENERATED -->

## Architecture

```
src/
├── lib.rs              # PyO3 module + StockpileScanner
├── bin/fs-ocr.rs       # CLI binary (clap)
├── constants.rs        # Hardcoded values / resolution scaling
├── error.rs            # Error types
├── config.rs           # ScanConfig
├── image_utils.rs      # RGB→grayscale, crop helpers
├── models/             # Output structs
│   ├── stockpile.rs
│   ├── stockpile_item.rs
│   └── timing.rs       # Per-stage Timing
├── enums/              # Type enums
│   ├── stockpile_type.rs
│   ├── item_faction.rs
│   └── item_category.rs
├── detector/           # ROI + grey mask detection
│   ├── black_box.rs    # Dark ROI localization (first pass)
│   ├── geometry.rs
│   └── grey_mask.rs
├── template/           # Template matching
│   ├── database.rs     # HDF5 loading
│   ├── matching.rs     # NCC + adaptive escalation + tiebreaker
│   └── phash.rs        # Perceptual hashing
├── ocr/                # Text + quantity extraction
│   ├── engine.rs       # OcrEngine trait + OcrConfig
│   ├── basic.rs        # OcrsEngine (pure-Rust ocrs)
│   ├── digit_matcher.rs# Glyph-template digit recognition (quantities)
│   ├── preprocess.rs   # Grayscale/upscale/threshold
│   ├── quantity.rs     # Quantity parsing + validation
│   └── tesseract.rs    # Tesseract backend (feature: ocr-full)
└── coordinator/        # Pipeline orchestration
    ├── pipeline.rs
    └── validation.rs
```

## License

[MIT](LICENSE)

# fs-ocr

Fast OCR library for Foxhole stockpile screenshots, written in Rust with Python bindings.

## Features

- **Fast Template Matching**: pHash filtering + NCC scoring with adaptive candidate escalation
- **ROI + Grey Mask Detection**: black-box ROI localization followed by grey-mask box detection
- **Quantity Recognition**: template-based glyph matching (no OCR engine needed for digits)
- **Pure-Rust OCR**: `ocrs`/`rten` with an embedded model — reads Latin + Russian and localized stockpile types; Chinese custom names via the optional system `tesseract` CLI (see [Language support](#language-support))
- **Python API + CLI**: PyO3 bindings (`import fs_ocr`) and an `fs-ocr` command-line tool

## Installation

### Standalone CLI (no Python required)

Prebuilt `fs-ocr` binaries are attached to each [GitHub Release](../../releases)
for Linux, Windows, and macOS (x86_64 + Apple Silicon). Download, extract, run:

```bash
tar -xzf fs-ocr-linux-x86_64.tar.gz   # or unzip the .zip on Windows
./fs-ocr scan screenshot.png -d templates.h5 --faction wardens
```

One static build is published per platform (`fs-ocr-<os>-<arch>`) — pure-Rust
OCR, no system dependencies. (The system `tesseract` CLI is used at runtime
only if present, for Chinese custom names.)

### From PyPI

```bash
pip install fs-ocr
```

OCR is pure Rust (`ocrs`/`rten`) with the recognition model embedded in the
wheel — **no system OCR libraries** are required to install or run.

> You still need a **template database** (`.h5`) to scan against; it is not
> bundled in the wheel. See [Template Database](#template-database).

### Language support

The embedded `ocrs` recognizer reads **Latin and Russian (Cyrillic)** text
natively, and the closed set of localized stockpile-type names (including
Chinese). No extra packages or language flags are needed for any of that.

The one exception is **free-form Chinese custom names**, which are read via the
**system `tesseract` CLI** if it is installed — detected at runtime, entirely
optional. If `tesseract` is absent, everything else still works and only the
Chinese custom name is left unread (no error).

| OS | Optional install (Chinese custom names) |
|----|------------------------------------------|
| Debian/Ubuntu | `sudo apt install tesseract-ocr tesseract-ocr-chi-sim` |
| Fedora/RHEL | `sudo dnf install tesseract tesseract-langpack-chi_sim` |
| macOS (Homebrew) | `brew install tesseract tesseract-lang` |
| Windows | [UB Mannheim installer](https://github.com/UB-Mannheim/tesseract/wiki) (select Chinese) or `choco install tesseract` |

Override the binary or language via `FS_OCR_TESSERACT` / `FS_OCR_TESSERACT_LANG`
(defaults `tesseract` / `chi_sim`).

### From Source

The build needs only a C/C++ toolchain — HDF5 is built from source via the
`static-hdf5` feature. No OpenCV or Tesseract dev libraries required:

```bash
# Build deps (Ubuntu/Debian)
sudo apt-get install cmake gcc g++ libclang-dev

# Build and install the Python module
pip install maturin
maturin develop --release

# Or build the standalone CLI (no Python / libpython linkage)
cargo build --release --no-default-features --bin fs-ocr
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

`scan_debug(image, ...)` and `scan_debug_file(path, ...)` take the same
arguments and return the same `Stockpile`, but additionally populate each
item's `debug_candidates` with the broad diagnostic candidate set (see
[StockpileItem](#stockpileitem)). The normal `scan`/`scan_file` output is
unchanged. Intended for tooling that needs to inspect *why* an icon matched.

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
item.code             # Item code or "Unknown"
item.quantity         # Detected quantity (-1 if failed)
item.crated           # Whether item is crated
item.confidence       # Match confidence (0.0 - 1.0)
item.x, item.y        # Icon top-left (px) in the source screenshot
item.candidates       # Alternative matches (if confidence_gap > 0)
item.debug_candidates # Broad diagnostic set; None unless scanned via scan_debug
```

### DebugCandidate

Populated only by `scan_debug` / `scan_debug_file`. One entry per template that
passed the icon's pHash threshold for its crated state — any
code/category/mod/faction — NCC-scored and ranked descending, capped by
`max_ncc_candidates`. The item's `code`/`confidence` equals the top candidate.

```python
candidate.code           # Item code
candidate.confidence     # NCC score (0.0 - 1.0)
candidate.mod            # Mod name (e.g. "vanilla", "airborne")
candidate.category       # "item" / "vehicle" / "shippable" / "invalid"
candidate.crated         # Whether the template is crated
candidate.faction        # "neutral" / "Colonials" / "Wardens"
candidate.phash_distance # Hamming distance icon-to-template (lower = closer)
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

## Template Database

`fs-ocr` matches icons against a pre-built HDF5 template database, supplied at
runtime via the CLI `--database` flag or the `StockpileScanner(database_path=...)`
argument. It is **not bundled** with the wheel or the CLI binary.

The database is generated separately from game assets by the
[foxhole-stockpiles](https://github.com/xurxogr/foxhole-stockpiles) project. Grab
the `.h5` file from its [`data/`](https://github.com/xurxogr/foxhole-stockpiles/tree/main/data)
directory and pass its path via `--database` / `database_path`.

```bash
fs-ocr scan screenshot.png -d fs_airborne.h5
```

## Development

<!-- AUTO-GENERATED from Cargo.toml + pyproject.toml -->
### Commands

| Command | Description |
|---------|-------------|
| `cargo test` | Run the Rust test suite |
| `cargo clippy --all-targets -- -D warnings` | Run linter (warnings as errors) |
| `cargo fmt` | Format code with rustfmt |
| `cargo build --release` | Build optimized library |
| `cargo build --release --no-default-features --bin fs-ocr` | Build the standalone CLI (no libpython) |
| `maturin develop --release` | Build and install Python module (dev) |
| `maturin build --release` | Build Python wheel for distribution |

### Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `python` | **on** | PyO3 + numpy bindings; drop with `--no-default-features` for a pure CLI |
| `static-hdf5` | off | Build/statically link libhdf5 from source (used by CI wheels) |

### Requirements

- Rust toolchain (edition 2021)
- Python 3.10+ (for bindings)
- Build tools: `cmake`, `gcc`/`g++`, `libclang-dev`
- Optional runtime: system `tesseract` CLI (Chinese custom names only)

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
│   └── grey_mask/      # detector + morphology + grouping
├── template/           # Template matching
│   ├── database.rs     # HDF5 loading
│   ├── matching.rs     # NCC + adaptive escalation + tiebreaker
│   ├── phash.rs        # Perceptual hashing
│   └── {label,public,type}_match.rs  # embedded-asset template matchers
├── ocr/                # Text + quantity extraction
│   ├── engine.rs       # OcrEngine trait + OcrConfig
│   ├── basic.rs        # OcrsEngine (pure-Rust ocrs)
│   ├── digit_matcher.rs# Glyph-template digit recognition (quantities)
│   ├── preprocess.rs   # upscale helpers
│   ├── quantity.rs     # Quantity validation
│   └── tesseract.rs    # ChineseNameReader (system `tesseract` CLI)
└── coordinator/        # Pipeline orchestration
    ├── pipeline.rs
    ├── region_preprocess.rs # OCR image prep (luma/contrast/framing/upscale)
    ├── metadata_parse.rs    # shard / timestamp / public-name parsing
    └── validation.rs
```

## License

[MIT](LICENSE)

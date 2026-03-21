# fs-ocr

Fast OCR library for Foxhole stockpile screenshots, written in Rust with Python bindings.

## Features

- **Fast Template Matching**: Two-phase matching using pHash filtering and NCC scoring
- **Grey Mask Detection**: HSV+RGB dual-mask approach for robust grey box detection
- **Quantity OCR**: Custom Tesseract model for game-specific number recognition
- **Python API**: Easy-to-use Python bindings via PyO3

## Installation

### From PyPI (when published)

```bash
pip install fs-ocr
```

### From Source

Requires Rust toolchain and system dependencies:

```bash
# Install system dependencies (Ubuntu/Debian)
sudo apt-get install libhdf5-dev libopencv-dev libtesseract-dev libleptonica-dev libclang-dev

# Build and install
pip install maturin
maturin develop
```

## Python Usage

```python
from fs_ocr import StockpileScanner, ScanConfig
import numpy as np

# Create scanner with database path
scanner = StockpileScanner(database_path="templates.h5")

# Scan from NumPy array (H x W x 3, uint8, RGB)
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
    tessdata_path: str = "tessdata"  # Path to Tesseract data directory
)

result = scanner.scan(
    image: np.ndarray,       # RGB image (H x W x 3)
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
    phash_threshold: int = 15,         # Max Hamming distance for pHash filter (lower=faster)
    max_ncc_candidates: int = 50,      # Max templates to run NCC on after pHash filter
    confidence_gap: float = 0.0,       # Return alternatives within this gap of best match
    ncc_tiebreaker_threshold: float = 0.0015  # Edge-based tiebreaker for close matches
)

# Serialize/deserialize
config.to_json() -> str
ScanConfig.from_json(json_str) -> ScanConfig
```
<!-- END AUTO-GENERATED -->

### Stockpile

Scan result containing detected items.

```python
stockpile.name           # Custom stockpile name (if applicable)
stockpile.stockpile_type # StockpileType enum
stockpile.items          # List[StockpileItem]
stockpile.timestamp      # ISO 8601 timestamp
stockpile.shard          # Game shard name
stockpile.resolution     # Screenshot resolution
stockpile.errors         # List of error messages
stockpile.to_json()      # Serialize to JSON
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

## Building the Template Database

The template database is built from game assets. See the main foxhole-stockpiles repository for details on generating the HDF5 database.

## Development

<!-- AUTO-GENERATED from Cargo.toml + pyproject.toml -->
### Commands

| Command | Description |
|---------|-------------|
| `cargo test` | Run Rust test suite (46 tests) |
| `cargo clippy` | Run linter with warnings |
| `cargo build --release` | Build optimized library |
| `maturin develop` | Build and install Python module (dev) |
| `maturin build --release` | Build Python wheel for distribution |

### Requirements

- Rust 1.70+ (edition 2021)
- Python 3.10+ (for bindings)
- System libraries: `libhdf5-dev`, `libtesseract-dev`, `libleptonica-dev`, `libclang-dev`

### Dev Dependencies

```bash
pip install pytest numpy  # Python dev deps
```
<!-- END AUTO-GENERATED -->

## Architecture

```
src/
├── lib.rs              # PyO3 module entry
├── constants.rs        # Hardcoded values
├── error.rs            # Error types
├── config.rs           # ScanConfig
├── models/             # Output structs
│   ├── stockpile.rs
│   └── stockpile_item.rs
├── enums/              # Type enums
│   ├── stockpile_type.rs
│   ├── item_faction.rs
│   └── item_category.rs
├── detector/           # Grey mask detection
│   ├── geometry.rs
│   └── grey_mask.rs
├── template/           # Template matching
│   ├── database.rs     # HDF5 loading
│   ├── matching.rs     # NCC matching
│   └── phash.rs        # Perceptual hashing
├── ocr/                # Text extraction
│   ├── tesseract.rs
│   └── quantity.rs
└── coordinator/        # Pipeline orchestration
    ├── pipeline.rs
    └── validation.rs
```

## License

MIT

<!-- Generated: 2026-03-21 | Files scanned: 24 | Token estimate: ~600 -->

# fs-ocr Architecture

## Overview

Rust library (PyO3) for OCR of Foxhole game stockpile screenshots.
Extracts item codes, quantities, and metadata via template matching + Tesseract.

## Data Flow

```
Image (RGB)
    │
    ▼
┌─────────────────┐
│ GreyMaskDetector│  Grey pixel detection → quantity box positions
└────────┬────────┘
         │
    ┌────┴────┬─────────────┐
    ▼         ▼             ▼
┌────────┐ ┌────────┐  ┌─────────┐
│Quantity│ │Template│  │Metadata │
│  OCR   │ │Matching│  │  OCR    │
│(Tess.) │ │(pHash+ │  │(type,   │
│        │ │  NCC)  │  │name,    │
└────┬───┘ └────┬───┘  │shard)   │
     │          │      └────┬────┘
     └────┬─────┴───────────┘
          ▼
    ┌───────────┐
    │ Stockpile │  JSON-serializable result
    └───────────┘
```

## Module Structure

```
src/
├── lib.rs              # PyO3 module + StockpileScanner class
├── config.rs           # ScanConfig (thresholds, tuning)
├── constants.rs        # Resolution scaling, supported resolutions
├── error.rs            # FsOcrError enum (thiserror)
├── coordinator/
│   ├── pipeline.rs     # ScanPipeline orchestration (660 lines)
│   └── validation.rs   # Quantity descending-order checks
├── detector/
│   ├── geometry.rs     # Bounding box, contour extraction
│   └── grey_mask.rs    # Grey pixel detection (530 lines)
├── enums/
│   ├── item_faction.rs # Wardens/Colonials/Neutral
│   ├── item_category.rs# Item/Vehicle/Structure/etc.
│   └── stockpile_type.rs # Base types (Seaport, Depot, etc.)
├── models/
│   ├── stockpile.rs    # Top-level scan result
│   └── stockpile_item.rs # Individual item match
├── ocr/
│   ├── quantity.rs     # Parse "1,234" or "1.2k"
│   └── tesseract.rs    # TextExtractor wrapper (500 lines)
└── template/
    ├── database.rs     # HDF5 template loading (700 lines)
    ├── matching.rs     # NCC + tiebreaker logic (400 lines)
    └── phash.rs        # Perceptual hash (aHash, 64-bit)
```

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| pyo3/numpy | Python bindings |
| hdf5 | Template database I/O |
| leptess | Tesseract OCR wrapper |
| rayon | Parallel NCC matching |
| image | File loading (scan_file) |
| serde_json | JSON serialization |

## Performance Notes

- pHash pre-filters templates (Hamming distance ≤ threshold)
- NCC uses precomputed template stats (mean, inv_std)
- Per-row quantity OCR parallelized via thread-local Tesseract
- Template DB cached per resolution

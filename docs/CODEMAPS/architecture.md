<!-- Generated: 2026-05-21 | Files scanned: 31 | Token estimate: ~700 -->

# fs-ocr Architecture

## Overview

Rust library (PyO3) + CLI for OCR of Foxhole game stockpile screenshots.
Extracts item codes, quantities, and metadata via template matching.
Two consumers: `fs_ocr` Python module (cdylib) and `fs-ocr` CLI binary.

## Data Flow

```
Image (RGB)
    │
    ▼
┌──────────────────┐
│ BlackBoxDetector │  Find dark stockpile ROI (fast first pass)
└────────┬─────────┘
         ▼
┌──────────────────┐
│ GreyMaskDetector │  Grey detection on ROI → quantity box positions, groups
└────────┬─────────┘
         │
    ┌────┴────┬──────────────┐
    ▼         ▼              ▼
┌────────┐ ┌────────┐  ┌──────────┐
│Quantity│ │Template│  │Metadata  │
│ digit  │ │Matching│  │OCR       │
│matcher │ │(pHash+ │  │(type/name│
│(glyph  │ │ NCC +  │  │via ocrs/ │
│ tmpl)  │ │ adapt) │  │tesseract;│
└────┬───┘ └────┬───┘  │shard via │
     │          │      │ ocrs)    │
     │          │      └────┬─────┘
     └────┬─────┴───────────┘
          ▼
    ┌───────────┐
    │ Stockpile │  JSON-serializable result (+ optional Timing)
    └───────────┘
```

## Module Structure

```
src/
├── lib.rs              # PyO3 module + StockpileScanner class (531 lines)
├── bin/fs-ocr.rs       # CLI binary (clap; scan/version subcommands)
├── config.rs           # ScanConfig (thresholds, adaptive NCC tuning)
├── constants.rs        # Resolution scaling, supported resolutions
├── error.rs            # FsOcrError enum (thiserror)
├── image_utils.rs      # RGB→grayscale, crop helpers
├── coordinator/
│   ├── pipeline.rs     # ScanPipeline orchestration (1111 lines)
│   └── validation.rs   # Quantity descending-order checks
├── detector/
│   ├── black_box.rs    # Dark ROI localization (first pass)
│   ├── geometry.rs     # Bounding box, contour extraction
│   └── grey_mask.rs    # Grey pixel detection + grouping (1261 lines)
├── enums/
│   ├── item_faction.rs # Neutral/Colonials/Wardens
│   ├── item_category.rs# Item/Vehicle/Structure/Shippable/Liquid
│   └── stockpile_type.rs # Base types (Seaport, Depot, etc.)
├── models/
│   ├── stockpile.rs    # Top-level scan result
│   ├── stockpile_item.rs # Item match + ItemCandidate
│   └── timing.rs       # Per-stage Timing metrics
├── ocr/
│   ├── engine.rs       # OcrEngine trait + OcrConfig
│   ├── basic.rs        # OcrsEngine (pure-Rust ocrs backend)
│   ├── digit_matcher.rs# Template-based glyph digit recognition (843 lines)
│   ├── preprocess.rs   # Grayscale/upscale/threshold for OCR
│   ├── quantity.rs     # Parse "1,234" / "1.2k"; descending checks
│   └── tesseract.rs    # TextExtractor (only with `ocr-full` feature)
└── template/
    ├── database.rs     # HDF5 template loading (718 lines)
    ├── matching.rs     # NCC + adaptive escalation + tiebreaker (557 lines)
    └── phash.rs        # Perceptual hash (aHash, 64-bit)
```

## OCR Backends

| Backend | Default | Used for | Feature |
|---------|---------|----------|---------|
| ocrs (pure Rust) | yes | shard, timestamp, type/name fallback | always built |
| digit_matcher (glyph templates) | yes | quantity boxes (primary) | always built |
| Tesseract (leptess) | no | multilingual type/name | `ocr-full` |

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| pyo3/numpy 0.24 | Python bindings |
| hdf5 0.8 | Template database I/O |
| ocrs/rten 0.11/0.22 | Pure-Rust OCR |
| rayon | Parallel NCC matching |
| clap 4 | CLI parsing |
| image 0.25 | File / stdin decoding |
| serde_json | JSON serialization |
| leptess (optional) | Tesseract OCR |

## Performance Notes

- BlackBox ROI pass crops work area before grey-mask scan
- Quantities resolved by glyph template matching (no OCR engine call)
- pHash pre-filters templates (Hamming distance ≤ threshold)
- NCC adaptive escalation: 25 → 50 → 100 candidates until conf ≥ 0.90
- NCC uses precomputed template stats (mean, inv_std), parallel via Rayon
- Template DB cached per resolution; `preload()` warms caches

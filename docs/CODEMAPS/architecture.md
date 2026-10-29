<!-- Generated: 2026-06-23 | Files scanned: 42 | Token estimate: ~770 -->

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
│ digit  │ │Matching│  │type/name/│
│matcher │ │(pHash+ │  │shard/time│
│(glyph  │ │ NCC +  │  │(ocrs +   │
│ tmpl)  │ │ adapt) │  │ CN via   │
│        │ │        │  │ tesseract│
└────┬───┘ └────┬───┘  │ CLI)     │
     │          │      └────┬─────┘
     └────┬─────┴───────────┘
          ▼
    ┌───────────┐
    │ Stockpile │  JSON-serializable result (+ optional Timing)
    └───────────┘
```

## Public API surface (lib.rs)

Only 5 modules are `pub` (the intentional API); everything else is `mod`
(crate-internal). Public: `config` (ScanConfig), `coordinator` (ScanPipeline),
`enums` (ItemFaction/ItemCategory/StockpileType/GameLanguage), `error`
(FsOcrError/Result), `models` (Stockpile + friends). Internal: `constants`,
`detector`, `image_utils`, `ocr`, `template`, `text_utils`.

## Module Structure

```
src/
├── lib.rs              # PyO3 module + StockpileScanner class; scan/scan_debug (433)
├── bin/fs-ocr.rs       # CLI binary (clap; scan/version subcommands)
├── config.rs           # ScanConfig (thresholds, adaptive NCC tuning)
├── constants.rs        # Resolution scaling, supported resolutions
├── error.rs            # FsOcrError enum (thiserror)
├── image_utils.rs      # RGB→grayscale, crop helpers
├── text_utils.rs       # Levenshtein / fuzzy string helpers
├── coordinator/
│   ├── pipeline.rs         # ScanPipeline orchestration; scan_with/match_icons[_debug] (1190)
│   ├── region_preprocess.rs# OCR image prep: luma, autocontrast, framing,
│   │                       #   upscale, name-row split/join (857)
│   ├── metadata_parse.rs   # client-lang routing, public-default name,
│   │                       #   shard match, timestamp day/hour (290)
│   ├── debug_ocr.rs        # FS_DEBUG_OCR=1 image dumps (env-gated)
│   └── validation.rs       # Quantity descending-order checks
├── detector/
│   ├── black_box.rs        # Dark ROI localization, first pass (420)
│   ├── geometry.rs         # BoundingRect, GroupInfo, DetectedRegions
│   └── grey_mask/          # (was grey_mask.rs, now a dir module)
│       ├── mod.rs          #   detector + detection orchestration (684)
│       ├── morphology.rs   #   dilate/erode/find_contours (266)
│       └── grouping.rs     #   box→grid grouping geometry (200)
├── enums/
│   ├── item_faction.rs     # Neutral/Colonials/Wardens
│   ├── item_category.rs    # Item/Vehicle/Shippable/Invalid
│   ├── game_language.rs    # English/German/French/Portuguese/Russian/Chinese
│   └── stockpile_type.rs   # Base types (Seaport, Depot, …)
├── models/
│   ├── stockpile.rs        # Top-level scan result
│   ├── stockpile_item.rs   # Item match + ItemCandidate + DebugCandidate
│   └── timing.rs           # Per-stage Timing metrics
├── ocr/
│   ├── engine.rs           # OcrEngine trait + OcrConfig
│   ├── basic.rs            # OcrsEngine (pure-Rust ocrs backend)
│   ├── mod.rs              # TextExtractor (ocrs recognizer wrapper)
│   ├── digit_matcher.rs    # Template-based glyph digit recognition (843)
│   ├── preprocess.rs       # upscale_bilinear for OCR
│   ├── quantity.rs         # descending-order checks
│   └── tesseract.rs        # ChineseNameReader (system `tesseract` CLI)
└── template/
    ├── database.rs         # HDF5 template loading (546)
    ├── matching.rs         # NCC + adaptive escalation + tiebreaker; match_icon_debug (681)
    ├── phash.rs            # Perceptual hash (aHash, 64-bit)
    ├── label_match.rs      # generic template label matcher
    ├── public_match.rs     # "Public" default-name template (embedded asset)
    └── type_match.rs       # stockpile-type template (embedded asset)
```

## Debug candidate path

`scan_debug`/`scan_debug_file` reuse the full pipeline (detection, quantity,
metadata) and swap only the matching step: `scan_with(debug=true)` calls
`match_icons_debug`, which per icon returns the broad pHash-filtered,
NCC-scored candidate set (crated-only filter — no faction/category/mod), each
with `phash_distance`, attached as `StockpileItem.debug_candidates`. The
production `scan` path and its JSON are unchanged (field serde-skipped when
`None`). Intended for fs-tools' debug image viewer.

## OCR Backends

| Backend | Used for | Notes |
|---------|----------|-------|
| ocrs (pure Rust) | shard, timestamp, type/name (Latin + Cyrillic) | always built; recognition model embedded via `include_bytes!` |
| digit_matcher (glyph templates) | quantity boxes (primary) | always built; no OCR engine call |
| `tesseract` CLI (runtime) | Chinese custom names only | optional system tool, probed at runtime; absent → that feature degrades |

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| pyo3/numpy 0.24 | Python bindings (abi3-py310) |
| hdf5-metno 0.15 / hdf5-metno-sys 0.12 (as `hdf5` / `hdf5-sys`) | Template database I/O |
| ocrs 0.11 / rten 0.22 | Pure-Rust OCR |
| rayon 1.10 | Parallel NCC matching |
| clap 4 | CLI parsing |
| image 0.25 | File / stdin decoding (png/jpeg/bmp/gif/webp/tiff) |
| serde_json 1.0 | JSON serialization |

## Performance Notes

- BlackBox ROI pass crops work area before grey-mask scan
- Quantities resolved by glyph template matching (no OCR engine call)
- pHash pre-filters templates (Hamming distance ≤ threshold)
- NCC adaptive escalation: 25 → 50 → 100 candidates until conf ≥ threshold
- NCC uses precomputed template stats (mean, inv_std), parallel via Rayon
- Template DB cached per resolution; `preload()` warms caches
- Blank/contrast-free OCR regions short-circuit to empty (skip the model)

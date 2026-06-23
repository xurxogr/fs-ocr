<!-- Generated: 2026-06-23 | Files scanned: 42 | Token estimate: ~840 -->

# fs-ocr Internals

## Pipeline Stages (ScanPipeline::scan)

### 0. ROI Localization (BlackBoxDetector)

```
Input: RGB image
Process:
  1. Sample rows for horizontal runs of dark pixels (< BLACK_THRESHOLD=15)
  2. Match runs to VALID_WIDTHS_2160 (scaled): 600/612/808/1004/1200
  3. Return bounding ROI (with padding) + scale factor
Output: BlackBoxResult { roi, scale_factor }  (falls back to full image if none)
```

### 1. Detection (GreyMaskDetector)

```
Input: RGB image cropped to ROI (detect_roi_fast)
Process:
  1. Grey mask: R,G,B near-equal, value in [GRAY_LOWER=15, GRAY_UPPER=98]
  2. Morphological close (kernel 3) then open (kernel 5)
  3. Connected components → contours; filter by scaled box size
  4. Group boxes by Y into rows/groups (GROUP_OFFSET separation)
  5. Compute icon regions (offset left of quantity box)
  6. Locate type/name/shard regions relative to first icon
Output: DetectedRegions { quantity_boxes, icon_regions, groups,
                          type_region, name_region, shard_region, info_bar_height }
Guard: MAX_TOTAL_BOXES=200 (DoS protection)
```

### 2. Quantity Recognition (digit_matcher — primary)

```
Input: grayscale image + quantity_boxes
Process (recognize_quantities_batch):
  1. Crop each box, normalize glyph height to TEMPLATE_HEIGHT=24px
  2. Match each glyph against bit-packed 0-9 Renner templates
  3. Accept digit if score ≥ MIN_MATCH_SCORE (0.6; relaxed 0.45 below 0.75 scale)
  4. Assemble digits into integer; handle "k" ×1000
Output: Vec<i32> quantities (-1 on failure)
Note: pure template matching — no OCR engine invoked for quantities.
Validation: validate_descending_order() flags non-descending quantities.
```

### 3. Template Matching (TemplateMatcher)

```
Input: icon images, template database
Process:
  1. Compute pHash per icon (8×8 aHash, 64-bit)
  2. Filter candidates by Hamming distance ≤ phash_threshold (15)
  3. Adaptive NCC escalation:
       score ncc_initial_candidates (25) by NCC
       if best confidence < ncc_escalation_threshold (0.90):
         double candidate count (→50→100, capped at max_ncc_candidates)
         reuse already-computed scores
  4. Tiebreaker: if top NCC gap ≤ ncc_tiebreaker_threshold (0.003),
     pick winner by edge(Sobel)-diff
  5. Group-based category detection (first group may skip filter)
Output: MatchResult { best_match, confidence, gap_candidates }
```

### 3b. Debug Matching (TemplateMatcher::match_icon_debug — scan_debug only)

```
Input: icon images, template database, icon crated state
Process (match_icons_debug, parallel per icon):
  1. Candidate filter: crated state ONLY (no faction/category/mod)
  2. pHash filter (Hamming ≤ phash_threshold), capped at max_ncc_candidates
  3. NCC-score every survivor (no adaptive escalation, no tiebreaker)
  4. Sort by NCC desc; record phash_distance per candidate
Output: Vec<DebugMatch>{code,mod,category,crated,faction,confidence,phash_distance}
        → item.code/confidence = top candidate; full set on debug_candidates
Note: shares extract_icon_phash with the production path; production scan
      output is unchanged (debug_candidates serde-skipped when None).
```

### 4. Metadata Extraction

```
Regions: type_region, name_region, shard_region
Process:
  - Type: ocrs recognizer + type-template match (type_templates.bin)
          → StockpileType + GameLanguage
  - Name: ocrs recognizer for Latin/Cyrillic; Chinese custom names read
          via the system `tesseract` CLI when installed (runtime probe).
          "Public" default detected via public_templates.bin template match.
  - Shard + in-game timestamp: ocrs (Latin), timestamp via metadata_parse
          (client-language time mask → day + HH:MM)
  - name is a non-public custom name → is_reserved = true
```

## Key Algorithms

### pHash (Perceptual Hash)

```rust
fn compute_phash(bgr, w, h) -> u64 {
    gray   = bgr_to_grayscale(bgr)        // OpenCV luma formula
    resized= resize_inter_area(gray, 8, 8)
    avg    = mean(resized)
    hash   = bits where pixel > avg       // 64 bits, MSB first
}
```

### NCC (Normalized Cross-Correlation)

```rust
fn ncc_with_precomputed(icon, template, tmpl_mean, tmpl_inv_std) -> f32 {
    icon_mean = mean(icon)
    cross_sum = Σ (icon[i]-icon_mean) * (template[i]-tmpl_mean)
    icon_std  = sqrt(Σ (icon[i]-icon_mean)²)
    cross_sum * tmpl_inv_std / icon_std   // -1.0 .. 1.0
}
```

### Tiebreaker (Edge Difference)

```rust
// Sobel mixed derivative d²f/dxdy, kernel [1,0,-1; 0,0,0; -1,0,1]
edge_diff = mean_abs(sobel_xy(img1) - sobel_xy(img2))  // lower = better
```

## Template Database (HDF5)

```
database.hdf5
├── Attributes: version, format, resolutions=["664",...,"2160"]
└── /{resolution}/
    ├── Attributes: resolution, template_count, icon_size, version
    └── Datasets:
        ├── images:   (N, H, W, 3) uint8
        ├── codes:    VarLenUnicode (item codes)
        ├── mods:     VarLenUnicode (mod names)
        ├── crated:   bool
        ├── faction:  uint8
        ├── category: uint8
        └── phash:    uint64
```

## Error Types (FsOcrError)

```rust
enum FsOcrError {
    Image(String),        // Invalid image format/size
    Database(String),     // HDF5 load failure
    Ocr(String),          // OCR error
    NoStockpileDetected,  // No grey boxes found
}
```

## Threading Model

- Main thread: ROI/grey detection, quantity matching, metadata OCR
- Rayon pool: NCC matching parallel over candidate templates
- OcrEngine trait is Send + Sync

## Resolution Handling

- BASE_RESOLUTION = 2160; all layout constants scale by height/2160
- SUPPORTED_RESOLUTIONS: 16 heights (664..2160); find_closest_resolution()
  snaps the screenshot height to the nearest DB group

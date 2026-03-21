<!-- Generated: 2026-03-21 | Files scanned: 24 | Token estimate: ~700 -->

# fs-ocr Internals

## Pipeline Stages

### 1. Detection (GreyMaskDetector)

```
Input: RGB image (H×W×3)
Process:
  1. Create grey mask (R,G,B within ±15 of each other, brightness 20-200)
  2. Morphological close (dilate→erode) to fill gaps
  3. Find connected components (flood fill contours)
  4. Filter by expected box size (resolution-scaled)
  5. Group boxes by Y-coordinate into rows
  6. Compute icon regions (offset left of quantity box)
Output: DetectedRegions { quantity_boxes, icon_regions, groups, type_region, name_region, shard_region }
```

### 2. Quantity OCR (TextExtractor + quantity parser)

```
Input: Quantity box regions
Process:
  1. Build row composite images (boxes in same row concatenated)
  2. Preprocess: grayscale → upscale → Otsu threshold → dilate
  3. Tesseract OCR (renner_numbers model, PSM 7)
  4. Parse text: handle commas, "k" suffix (×1000)
Output: Vec<i32> quantities (-1 if parse failed)
```

### 3. Template Matching (TemplateMatcher)

```
Input: Icon images, template database
Process:
  1. Compute pHash for each icon (8×8 aHash)
  2. Filter candidates by Hamming distance ≤ threshold
  3. Run NCC on top candidates (parallelized via Rayon)
  4. Apply tiebreaker (edge-based diff) for close matches
  5. Group-based category detection (first N items without filter)
Output: MatchResult { best_match, confidence, top_matches, gap_candidates }
```

### 4. Metadata OCR

```
Regions: type_region, name_region, shard_region
Process:
  - Type: Tesseract eng PSM 7 → StockpileType::from_string()
  - Name: Tesseract eng PSM 7 (extra upscale for small text)
  - Shard: Tesseract eng PSM 6 (block mode for multi-line)
```

## Key Algorithms

### pHash (Perceptual Hash)

```rust
fn compute_phash(bgr: &[u8], w: usize, h: usize) -> u64 {
    grayscale = bgr_to_grayscale(bgr)      // OpenCV formula
    resized = resize_inter_area(gray, 8, 8) // Area interpolation
    avg = mean(resized)
    hash = bits where pixel > avg           // 64 bits, MSB first
}
```

### NCC (Normalized Cross-Correlation)

```rust
fn ncc_with_precomputed(icon: &[u8], template: &[u8], tmpl_mean: f32, tmpl_inv_std: f32) -> f32 {
    icon_mean = mean(icon)
    cross_sum = Σ (icon[i] - icon_mean) * (template[i] - tmpl_mean)
    icon_std = sqrt(Σ (icon[i] - icon_mean)²)
    return cross_sum * tmpl_inv_std / icon_std  // Range: -1.0 to 1.0
}
```

### Tiebreaker (Edge Difference)

```rust
fn compute_edge_diff(img1: &[u8], img2: &[u8], w: usize, h: usize) -> f32 {
    // Sobel mixed derivative d²f/dxdy (kernel: [1,0,-1; 0,0,0; -1,0,1])
    edges1 = sobel_xy(img1)
    edges2 = sobel_xy(img2)
    return mean_absolute_diff(edges1, edges2)  // Lower = better match
}
```

## Template Database (HDF5)

```
database.hdf5
├── Attributes: version=2, format="hdf5", resolutions=["664","720","1080",...]
└── /{resolution}/
    ├── Attributes: resolution, template_count, icon_size, version
    └── Datasets:
        ├── images: (N, H, W, 3) uint8
        ├── codes: VarLenUnicode (item codes)
        ├── mods: VarLenUnicode (mod names)
        ├── crated: bool
        ├── faction: uint8
        ├── category: uint8
        └── phash: uint64
```

## Error Types

```rust
enum FsOcrError {
    Image(String),           // Invalid image format/size
    Database(String),        // HDF5 load failure
    Ocr(String),             // Tesseract error
    NoStockpileDetected,     // No grey boxes found
}
```

## Threading Model

- Main thread: Detection, metadata OCR
- Rayon pool: NCC matching (parallel over candidates)
- Thread-local: Tesseract instances for quantity OCR (one per Rayon thread)

//! Main scanning pipeline orchestration.
//!
//! Coordinates the detection, OCR, and template matching stages
//! to produce a complete stockpile scan result.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Maximum total boxes to process (prevents DoS via excessive memory allocation).
/// A typical Foxhole stockpile has 6 columns × ~10 rows = 60 items max per view.
const MAX_TOTAL_BOXES: usize = 200;

/// Maximum boxes per row (game UI constraint is 6).
const MAX_BOXES_PER_ROW: usize = 20;

use rayon::prelude::*;

use crate::config::ScanConfig;
use crate::detector::{BlackBoxDetector, DetectedRegions, GreyMaskDetector};
use crate::enums::ItemFaction;
use crate::enums::StockpileType;
use crate::error::{FsOcrError, Result};
use crate::image_utils;
use crate::models::{ItemCandidate, Stockpile, StockpileItem};
use crate::ocr::tesseract::{preprocess_quantity_composite, TextExtractor};
use crate::template::database::TemplateDatabase;
use crate::template::matching::{MatchFilter, TemplateMatcher};
use crate::template::phash::compute_phash;

/// Main scanning pipeline for stockpile screenshots.
pub struct ScanPipeline {
    /// Template database path.
    database_path: String,
    /// Tessdata directory path.
    tessdata_path: String,
    /// Scan configuration.
    config: ScanConfig,
    /// Loaded template database (cached).
    database: Option<Arc<TemplateDatabase>>,
    /// Text extractor for quantity OCR (uses renner_numbers model).
    text_extractor: Option<TextExtractor>,
    /// Text extractor for single-line text OCR (type, name - PSM 7).
    text_extractor_eng: Option<TextExtractor>,
    /// Text extractor for multi-line text OCR (shard region - PSM 6).
    text_extractor_eng_block: Option<TextExtractor>,
}

impl ScanPipeline {
    /// Create a new scan pipeline.
    pub fn new<P: AsRef<Path>>(database_path: P, tessdata_path: P, config: ScanConfig) -> Self {
        Self {
            database_path: database_path.as_ref().to_string_lossy().to_string(),
            tessdata_path: tessdata_path.as_ref().to_string_lossy().to_string(),
            config,
            database: None,
            text_extractor: None,
            text_extractor_eng: None,
            text_extractor_eng_block: None,
        }
    }

    /// Ensure database and extractor are loaded.
    fn ensure_initialized(&mut self, resolution: i32) -> Result<()> {
        // Load database if not already loaded or resolution changed
        let needs_load = match &self.database {
            None => true,
            Some(db) => db.resolution != resolution,
        };

        if needs_load {
            let db = TemplateDatabase::load(&self.database_path, resolution)?;
            self.database = Some(Arc::new(db));
        }

        // Initialize text extractors
        if self.text_extractor.is_none() {
            match TextExtractor::new(&self.tessdata_path, "renner_numbers") {
                Ok(extractor) => self.text_extractor = Some(extractor),
                Err(e) => {
                    // Log but don't fail - OCR is optional
                    eprintln!("Warning: Failed to initialize quantity OCR: {}", e);
                }
            }
        }

        // Initialize English text extractor for single-line text (type, name - PSM 7)
        if self.text_extractor_eng.is_none() {
            // Use empty path to let Tesseract find its default tessdata
            match TextExtractor::new_for_text_default("eng") {
                Ok(extractor) => self.text_extractor_eng = Some(extractor),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize text OCR: {}", e);
                }
            }
        }

        // Initialize English text extractor for multi-line text (shard region - PSM 6)
        if self.text_extractor_eng_block.is_none() {
            match TextExtractor::new_for_text_block_default("eng") {
                Ok(extractor) => self.text_extractor_eng_block = Some(extractor),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize block text OCR: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Preload the database and OCR engines for fast subsequent scans.
    ///
    /// Call this at server startup to avoid cold start on the first request.
    ///
    /// Args:
    ///     resolution: Target resolution height (e.g., 1080, 1440, 2160)
    pub fn preload(&mut self, resolution: i32) -> Result<()> {
        self.ensure_initialized(resolution)
    }

    /// Check if the scanner is preloaded.
    pub fn is_preloaded(&self) -> bool {
        self.database.is_some()
    }

    /// Scan a stockpile screenshot.
    ///
    /// Args:
    ///     image: RGB image data (row-major, 3 bytes per pixel)
    ///     width: Image width
    ///     height: Image height
    ///     faction: Optional faction filter for template matching
    ///
    /// Returns:
    ///     Complete stockpile scan result
    pub fn scan(
        &mut self,
        image: &[u8],
        width: i32,
        height: i32,
        faction: Option<ItemFaction>,
    ) -> Result<Stockpile> {
        // Validate dimensions (prevent overflow and unreasonable sizes)
        const MAX_DIMENSION: i32 = 10_000;
        if width <= 0 || height <= 0 {
            return Err(FsOcrError::Image(format!(
                "Invalid dimensions: {}x{} (must be positive)",
                width, height
            )));
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(FsOcrError::Image(format!(
                "Dimensions too large: {}x{} (max: {}x{})",
                width, height, MAX_DIMENSION, MAX_DIMENSION
            )));
        }

        let resolution_str = format!("{}x{}", width, height);
        let mut stockpile = Stockpile::new(resolution_str, StockpileType::Undefined);

        // Validate image size with checked arithmetic to prevent overflow
        let expected_size = (width as i64)
            .checked_mul(height as i64)
            .and_then(|pixels| pixels.checked_mul(3))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or_else(|| {
                FsOcrError::Image(format!("Image dimensions overflow: {}x{}", width, height))
            })?;

        if image.len() != expected_size {
            return Err(FsOcrError::Image(format!(
                "Invalid image size: expected {}, got {}",
                expected_size,
                image.len()
            )));
        }

        // Initialize resources
        self.ensure_initialized(height)?;

        // Step 1: Detect quantity boxes and regions using hybrid black box + grey mask
        let detect_start = Instant::now();
        let (regions, blackbox_ms, greymask_ms) =
            match self.detect_stockpile_regions(image, width, height) {
                Ok(r) => r,
                Err(FsOcrError::NoStockpileDetected) => {
                    stockpile.add_error("No stockpile detected in image".to_string());
                    return Ok(stockpile);
                }
                Err(e) => return Err(e),
            };
        stockpile.timing_detection_ms = Some(detect_start.elapsed().as_secs_f64() * 1000.0);
        stockpile.timing_blackbox_ms = Some(blackbox_ms);
        stockpile.timing_greymask_ms = Some(greymask_ms);

        // Step 2: Extract quantities via OCR
        let quantity_start = Instant::now();
        let quantities = self.extract_quantities(image, width, height, &regions)?;
        stockpile.timing_quantity_ms = Some(quantity_start.elapsed().as_secs_f64() * 1000.0);

        // Step 3: Match icons to templates
        let match_start = Instant::now();
        let items = self.match_icons(image, width, height, &regions, &quantities, faction)?;
        stockpile.timing_matching_ms = Some(match_start.elapsed().as_secs_f64() * 1000.0);

        // Step 4: Extract stockpile metadata (type, name, shard)
        let metadata_start = Instant::now();
        self.extract_stockpile_metadata(image, width, height, &regions, &mut stockpile)?;
        stockpile.timing_metadata_ms = Some(metadata_start.elapsed().as_secs_f64() * 1000.0);

        // Build result
        for item in items {
            stockpile.add_item(item);
        }

        Ok(stockpile)
    }

    /// Extract stockpile type, name, and shard/timestamp via OCR.
    fn extract_stockpile_metadata(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        regions: &crate::detector::DetectedRegions,
        stockpile: &mut Stockpile,
    ) -> Result<()> {
        // Use English extractor for text (not numbers)
        let extractor = match &self.text_extractor_eng {
            Some(e) => e,
            None => return Ok(()), // No OCR available
        };

        let scale_factor = regions.scale_factor;

        // Extract stockpile type
        if let Some((x, y, w, h)) = regions.type_region {
            let type_img = extract_region(
                image,
                width as usize,
                height as usize,
                x.max(0) as usize,
                y.max(0) as usize,
                w as usize,
                h as usize,
            );

            // Preprocess for OCR (similar to Python's _prepare_image_for_detection)
            let (processed, proc_w, proc_h) =
                preprocess_for_text(&type_img, w as usize, h as usize, scale_factor);

            let text = extractor.extract_text(
                &processed,
                proc_w as i32,
                proc_h as i32,
                1, // grayscale
            )?;

            let stockpile_type = StockpileType::from_string(&text);
            stockpile.stockpile_type = stockpile_type;
        }

        // Extract shard and ingame timestamp (use block extractor for multi-line text)
        if let Some((x, y, w, h)) = regions.shard_region {
            // Use block extractor (PSM 6) for multi-line shard region
            let block_extractor = match &self.text_extractor_eng_block {
                Some(e) => e,
                None => extractor, // Fall back to single-line if block not available
            };

            let shard_img = extract_region(
                image,
                width as usize,
                height as usize,
                x.max(0) as usize,
                y.max(0) as usize,
                w as usize,
                h as usize,
            );

            // Preprocess for OCR (non-inverted for shard)
            let (processed, proc_w, proc_h) =
                preprocess_for_text_no_invert(&shard_img, w as usize, h as usize, scale_factor);

            let text = block_extractor.extract_text(
                &processed,
                proc_w as i32,
                proc_h as i32,
                1, // grayscale
            )?;

            // Parse shard and ingame timestamp
            let lines: Vec<&str> = text.lines().collect();
            if !lines.is_empty() {
                stockpile.ingame_timestamp = Some(extract_day_and_hour(lines[0]));
                if lines.len() > 1 {
                    stockpile.shard = Some(lines[1].trim().to_string());
                }
            }
        }

        // Extract stockpile name (only for types that support custom names)
        if stockpile.stockpile_type.has_custom_name() {
            if let Some((x, y, w, h)) = regions.name_region {
                let name_img = extract_region(
                    image,
                    width as usize,
                    height as usize,
                    x.max(0) as usize,
                    y.max(0) as usize,
                    w as usize,
                    h as usize,
                );

                // Preprocess with extra upscale for better name detection
                let (processed, proc_w, proc_h) =
                    preprocess_for_text_extra(&name_img, w as usize, h as usize, scale_factor);

                let text = extractor.extract_text(
                    &processed,
                    proc_w as i32,
                    proc_h as i32,
                    1, // grayscale
                )?;

                let name = text.trim();
                if !name.is_empty() {
                    stockpile.name = Some(name.to_string());
                }
            }
        }

        Ok(())
    }

    /// Extract quantities using parallel per-row OCR.
    fn extract_quantities(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        regions: &crate::detector::DetectedRegions,
    ) -> Result<Vec<i32>> {
        self.extract_quantities_per_row(image, width, height, regions)
    }

    /// Extract quantities using per-row composite images (parallelized).
    ///
    /// Builds ONE composite per row for OCR.
    /// Makes N parallel OCR calls (one per row) using thread-local Tesseract instances.
    fn extract_quantities_per_row(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        regions: &crate::detector::DetectedRegions,
    ) -> Result<Vec<i32>> {
        if self.text_extractor.is_none() {
            return Ok(vec![-1; regions.quantity_boxes.len()]);
        }

        let box_width = regions.box_width as usize;
        let box_height = regions.box_height as usize;
        let scale_factor = regions.scale_factor;

        if regions.quantity_boxes.is_empty() {
            return Ok(Vec::new());
        }

        // Security: Limit total boxes to prevent DoS via memory exhaustion
        if regions.quantity_boxes.len() > MAX_TOTAL_BOXES {
            return Err(FsOcrError::Image(format!(
                "Too many boxes detected: {} (max: {}). This may indicate a malformed image.",
                regions.quantity_boxes.len(),
                MAX_TOTAL_BOXES
            )));
        }

        // Group boxes by row
        let rows = self.group_boxes_by_row(regions);

        // Security: Validate row sizes before allocation
        for (row_idx, row) in rows.iter().enumerate() {
            if row.len() > MAX_BOXES_PER_ROW {
                return Err(FsOcrError::Image(format!(
                    "Row {} has too many boxes: {} (max: {})",
                    row_idx,
                    row.len(),
                    MAX_BOXES_PER_ROW
                )));
            }
        }

        // Step 1: Build all row composites (parallel)
        let row_data: Vec<(Vec<u8>, usize, usize, Vec<usize>)> = rows
            .par_iter()
            .filter(|row| !row.is_empty())
            .map(|row| {
                let gap = 20usize;
                let row_composite_width =
                    row.len() * box_width + (row.len().saturating_sub(1)) * gap;
                let mut row_composite = vec![0u8; row_composite_width * box_height * 3];

                let mut x_offset = 0usize;
                for &(_, qx, qy) in row {
                    for dy in 0..box_height {
                        for dx in 0..box_width {
                            let src_x = qx as usize + dx;
                            let src_y = qy as usize + dy;

                            if src_x < (width as usize) && src_y < (height as usize) {
                                let src_idx = (src_y * width as usize + src_x) * 3;
                                let dst_idx = (dy * row_composite_width + x_offset + dx) * 3;

                                if src_idx + 2 < image.len() && dst_idx + 2 < row_composite.len() {
                                    row_composite[dst_idx] = image[src_idx];
                                    row_composite[dst_idx + 1] = image[src_idx + 1];
                                    row_composite[dst_idx + 2] = image[src_idx + 2];
                                }
                            }
                        }
                    }
                    x_offset += box_width + gap;
                }

                // Preprocess
                let (processed, proc_w, proc_h) = preprocess_quantity_composite(
                    &row_composite,
                    row_composite_width,
                    box_height,
                    3,
                    scale_factor,
                );

                // Collect original indices
                let indices: Vec<usize> = row.iter().map(|&(idx, _, _)| idx).collect();

                (processed, proc_w, proc_h, indices)
            })
            .collect();

        // Step 2: Run OCR in parallel using thread-local Tesseract instances
        use std::cell::RefCell;
        thread_local! {
            static THREAD_EXTRACTOR: RefCell<Option<crate::ocr::tesseract::TextExtractor>> = const { RefCell::new(None) };
        }

        let tessdata_path = self.tessdata_path.clone();
        let ocr_results: Vec<(Vec<i32>, Vec<usize>)> = row_data
            .par_iter()
            .map(|(processed, proc_w, proc_h, indices)| {
                // Use thread-local Tesseract instance (lazy init)
                THREAD_EXTRACTOR.with(|cell| {
                    let mut extractor_opt = cell.borrow_mut();
                    if extractor_opt.is_none() {
                        *extractor_opt = crate::ocr::tesseract::TextExtractor::new(
                            &tessdata_path,
                            "renner_numbers",
                        )
                        .ok();
                    }

                    let extractor = match extractor_opt.as_ref() {
                        Some(e) => e,
                        None => return (vec![-1i32; indices.len()], indices.clone()),
                    };

                    let text = match extractor.extract_text(
                        processed,
                        *proc_w as i32,
                        *proc_h as i32,
                        1,
                    ) {
                        Ok(t) => t,
                        Err(_) => return (vec![-1i32; indices.len()], indices.clone()),
                    };

                    let parsed = crate::ocr::quantity::parse_quantity_text(&text);
                    let parsed_flat: Vec<i32> = parsed.into_iter().flatten().collect();

                    (parsed_flat, indices.clone())
                })
            })
            .collect();

        // Step 3: Merge results
        let mut quantities = vec![-1i32; regions.quantity_boxes.len()];
        for (parsed, indices) in ocr_results {
            for (i, &orig_idx) in indices.iter().enumerate() {
                if i < parsed.len() {
                    quantities[orig_idx] = parsed[i];
                }
            }
        }

        Ok(quantities)
    }

    /// Group quantity boxes by row (shared helper for both methods).
    fn group_boxes_by_row(
        &self,
        regions: &crate::detector::DetectedRegions,
    ) -> Vec<Vec<(usize, i32, i32)>> {
        let box_height = regions.box_height as usize;
        let row_tolerance = box_height / 2;
        let mut rows: Vec<Vec<(usize, i32, i32)>> = Vec::new();

        for (idx, &(qx, qy)) in regions.quantity_boxes.iter().enumerate() {
            let mut found_row = false;
            for row in &mut rows {
                if !row.is_empty() {
                    let row_y = row[0].2;
                    if (qy - row_y).unsigned_abs() as usize <= row_tolerance {
                        row.push((idx, qx, qy));
                        found_row = true;
                        break;
                    }
                }
            }
            if !found_row {
                rows.push(vec![(idx, qx, qy)]);
            }
        }

        // Sort rows by Y, and boxes within each row by X
        rows.sort_by_key(|row| row.first().map(|&(_, _, y)| y).unwrap_or(0));
        for row in &mut rows {
            row.sort_by_key(|&(_, x, _)| x);
        }

        rows
    }

    /// Match detected icons to templates with group-based category detection.
    ///
    /// For each group:
    /// 1. Match first N items without category filter (N=2 for first group, N=5 for others)
    /// 2. Detect most frequent category from matched items
    /// 3. Match remaining items with detected category filter
    ///
    /// This matches Python's behavior and improves accuracy for similar-looking items.
    fn match_icons(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        regions: &crate::detector::DetectedRegions,
        quantities: &[i32],
        faction: Option<ItemFaction>,
    ) -> Result<Vec<StockpileItem>> {
        use crate::enums::ItemCategory;
        use std::collections::HashMap;

        // If no database loaded, return unknown items
        let database = match &self.database {
            Some(db) => db,
            None => {
                return Ok(quantities
                    .iter()
                    .enumerate()
                    .map(|(i, &qty)| {
                        let crated = self.detect_crated_from_group(i, &regions.groups);
                        StockpileItem::unknown(qty, crated)
                    })
                    .collect());
            }
        };

        // Step 1: Extract all icon images and compute pHashes in parallel
        // Store (icon_data, phash, width, height) for each icon
        let icons_data: Vec<(Vec<u8>, u64, i32, i32)> = regions
            .icon_regions
            .par_iter()
            .map(|&(ix, iy, iw, ih)| {
                let icon_w = iw.max(1) as usize;
                let icon_h = ih.max(1) as usize;

                let icon_image = extract_region(
                    image,
                    width as usize,
                    height as usize,
                    ix.max(0) as usize,
                    iy.max(0) as usize,
                    icon_w,
                    icon_h,
                );

                let phash = compute_phash(&icon_image, icon_w, icon_h);
                (icon_image, phash, iw, ih)
            })
            .collect();

        // Create matcher
        let matcher = TemplateMatcher::new(
            Arc::clone(database),
            self.config.phash_threshold,
            self.config.max_ncc_candidates,
            self.config.confidence_gap,
            self.config.ncc_tiebreaker_threshold,
        );

        // Step 2: Process groups with category detection
        let total_items = icons_data.len();
        let mut items: Vec<StockpileItem> = vec![StockpileItem::unknown(-1, false); total_items];

        for (group_idx, group) in regions.groups.iter().enumerate() {
            // Number of items to match without category filter
            let filter_start = if group_idx == 0 { 2 } else { 5 };
            let unfiltered_count = filter_start.min(group.size);

            // Track detected categories
            let mut category_counts: HashMap<ItemCategory, usize> = HashMap::new();
            let mut detected_category: Option<ItemCategory> = None;

            for i in 0..group.size {
                let item_idx = group.start_index + i;
                if item_idx >= total_items {
                    break;
                }

                let (icon, phash, icon_w, icon_h) = &icons_data[item_idx];
                let quantity = quantities.get(item_idx).copied().unwrap_or(-1);

                // Use category filter after first N items
                let category = if i >= unfiltered_count {
                    detected_category
                } else {
                    None
                };

                // Match this icon
                let filter = MatchFilter::new().faction(faction).category(category);
                let result = matcher.match_icon_with_phash(icon, *icon_w, *icon_h, *phash, &filter);

                let item = match result {
                    Ok(match_result) if match_result.best_match.is_some() => {
                        let template = match_result.best_match.as_ref().unwrap();

                        // Track category for future items
                        if i < unfiltered_count {
                            *category_counts.entry(template.category).or_insert(0) += 1;

                            // Detect category after collecting enough samples
                            if category_counts.values().sum::<usize>() >= unfiltered_count {
                                detected_category = category_counts
                                    .iter()
                                    .max_by_key(|(_, count)| *count)
                                    .map(|(cat, _)| *cat);
                            }
                        }

                        // Convert gap candidates
                        let candidates = if match_result.gap_candidates.is_empty() {
                            None
                        } else {
                            Some(
                                match_result
                                    .gap_candidates
                                    .iter()
                                    .map(|(t, conf)| ItemCandidate::new(t.code.clone(), *conf))
                                    .collect(),
                            )
                        };

                        StockpileItem::new(
                            template.code.clone(),
                            quantity,
                            template.crated,
                            match_result.confidence,
                            candidates,
                        )
                    }
                    _ => {
                        let crated = self.detect_crated_from_group(item_idx, &regions.groups);
                        StockpileItem::unknown(quantity, crated)
                    }
                };

                items[item_idx] = item;
            }
        }

        Ok(items)
    }

    /// Detect if an item should be crated based on its group.
    fn detect_crated_from_group(
        &self,
        item_index: usize,
        groups: &[crate::detector::GroupInfo],
    ) -> bool {
        // First group is typically base items (not crated)
        // Later groups are typically crated items
        for (g, group) in groups.iter().enumerate() {
            if item_index >= group.start_index && item_index < group.start_index + group.size {
                // First group is not crated
                return g > 0;
            }
        }
        false
    }

    /// Get the scan configuration.
    pub fn config(&self) -> &ScanConfig {
        &self.config
    }

    /// Update the scan configuration.
    pub fn set_config(&mut self, config: ScanConfig) {
        self.config = config;
    }

    /// Get the database path.
    pub fn database_path(&self) -> &str {
        &self.database_path
    }

    /// Get the tessdata path.
    pub fn tessdata_path(&self) -> &str {
        &self.tessdata_path
    }

    /// Detect stockpile regions using hybrid approach: black box for ROI, then grey mask.
    ///
    /// Uses black box detection to find the Region of Interest (ROI), then runs
    /// grey mask detection only on the ROI for improved performance.
    /// Falls back to full-image detection if black box detection fails.
    ///
    /// Returns (regions, blackbox_ms, greymask_ms).
    fn detect_stockpile_regions(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
    ) -> Result<(DetectedRegions, f64, f64)> {
        // Step 1: Run black box detection to find ROI
        let bb_start = Instant::now();
        let bb_detector = BlackBoxDetector::new(width, height);
        let bb_result = bb_detector.detect(image, width, height)?;
        let blackbox_ms = bb_start.elapsed().as_secs_f64() * 1000.0;

        let roi = match bb_result {
            Some(r) => r.roi,
            None => {
                // Fall back to full-image grey mask detection
                let gm_start = Instant::now();
                let detector = GreyMaskDetector::new(width, height);
                let regions = detector.detect(image, width, height)?;
                let greymask_ms = gm_start.elapsed().as_secs_f64() * 1000.0;
                return Ok((regions, blackbox_ms, greymask_ms));
            }
        };

        let (roi_x, roi_y, roi_w, roi_h) = roi;

        // Create grey mask detector (reuse for all operations)
        let detector = GreyMaskDetector::new(width, height);

        // Validate ROI dimensions
        if roi_w <= 0 || roi_h <= 0 {
            let gm_start = Instant::now();
            let regions = detector.detect(image, width, height)?;
            let greymask_ms = gm_start.elapsed().as_secs_f64() * 1000.0;
            return Ok((regions, blackbox_ms, greymask_ms));
        }

        // Step 2: Run fast "not black" detection on ROI region.
        let gm_start = Instant::now();
        let mut regions =
            detector.detect_roi_fast(image, width, height, roi_x, roi_y, roi_w, roi_h)?;
        let greymask_ms = gm_start.elapsed().as_secs_f64() * 1000.0;

        // Step 3: Adjust coordinates back to original image space
        for (x, y) in &mut regions.quantity_boxes {
            *x += roi_x;
            *y += roi_y;
        }

        for (x, y, _, _) in &mut regions.icon_regions {
            *x += roi_x;
            *y += roi_y;
        }

        // Update vertical resolution to original image height
        regions.vertical_resolution = height;

        Ok((regions, blackbox_ms, greymask_ms))
    }
}

/// Extract a region from an RGB image.
fn extract_region(
    image: &[u8],
    img_width: usize,
    img_height: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut region = vec![0u8; width * height * 3];

    for dy in 0..height {
        for dx in 0..width {
            let src_x = x + dx;
            let src_y = y + dy;

            if src_x < img_width && src_y < img_height {
                let src_idx = (src_y * img_width + src_x) * 3;
                let dst_idx = (dy * width + dx) * 3;

                if src_idx + 2 < image.len() && dst_idx + 2 < region.len() {
                    region[dst_idx] = image[src_idx];
                    region[dst_idx + 1] = image[src_idx + 1];
                    region[dst_idx + 2] = image[src_idx + 2];
                }
            }
        }
    }

    region
}

/// Preprocess image region for text OCR (inverted for black text on white).
fn preprocess_for_text(
    image: &[u8],
    width: usize,
    height: usize,
    scale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Upscale factor: 2 / scale_factor to normalize to 2160p equivalent
    let upscale = 2.0 / scale_factor;
    let new_w = ((width as f64) * upscale) as usize;
    let new_h = ((height as f64) * upscale) as usize;

    // Convert RGB to grayscale
    let grayscale = image_utils::rgb_to_grayscale(image, width, height);

    // Simple nearest-neighbor upscale
    let upscaled = upscale_nearest(&grayscale, width, height, new_w, new_h);

    // Apply Otsu-like threshold with inversion
    let threshold = image_utils::compute_otsu_threshold(&upscaled);
    let binary: Vec<u8> = upscaled
        .iter()
        .map(|&v| if v < threshold { 255 } else { 0 })
        .collect();

    // Simple dilation (3x3 kernel)
    let dilated = dilate_3x3(&binary, new_w, new_h);

    (dilated, new_w, new_h)
}

/// Preprocess image region for text OCR (non-inverted for bright text on dark).
fn preprocess_for_text_no_invert(
    image: &[u8],
    width: usize,
    height: usize,
    scale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    let upscale = 2.0 / scale_factor;
    let new_w = ((width as f64) * upscale) as usize;
    let new_h = ((height as f64) * upscale) as usize;

    let grayscale = image_utils::rgb_to_grayscale(image, width, height);
    let upscaled = upscale_nearest(&grayscale, width, height, new_w, new_h);

    // Non-inverted threshold (white text on black background -> black text on white)
    let threshold = image_utils::compute_otsu_threshold(&upscaled);
    let binary: Vec<u8> = upscaled
        .iter()
        .map(|&v| if v > threshold { 255 } else { 0 })
        .collect();

    let dilated = dilate_3x3(&binary, new_w, new_h);

    (dilated, new_w, new_h)
}

/// Preprocess with extra upscale (2x more) for name detection.
fn preprocess_for_text_extra(
    image: &[u8],
    width: usize,
    height: usize,
    scale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    let upscale = 4.0 / scale_factor; // 2x extra upscale
    let new_w = ((width as f64) * upscale) as usize;
    let new_h = ((height as f64) * upscale) as usize;

    let grayscale = image_utils::rgb_to_grayscale(image, width, height);
    let upscaled = upscale_nearest(&grayscale, width, height, new_w, new_h);

    let threshold = image_utils::compute_otsu_threshold(&upscaled);
    let binary: Vec<u8> = upscaled
        .iter()
        .map(|&v| if v < threshold { 255 } else { 0 })
        .collect();

    let dilated = dilate_3x3(&binary, new_w, new_h);

    (dilated, new_w, new_h)
}

/// Simple nearest-neighbor upscale for grayscale image.
fn upscale_nearest(
    image: &[u8],
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; dst_w * dst_h];
    for y in 0..dst_h {
        for x in 0..dst_w {
            let src_x = (x * src_w) / dst_w;
            let src_y = (y * src_h) / dst_h;
            result[y * dst_w + x] = image[src_y * src_w + src_x];
        }
    }
    result
}

/// Simple 3x3 dilation for binary image.
fn dilate_3x3(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut max_val = 0u8;
            for dy in 0i32..3 {
                for dx in 0i32..3 {
                    let ny = (y as i32 + dy - 1).max(0).min(height as i32 - 1) as usize;
                    let nx = (x as i32 + dx - 1).max(0).min(width as i32 - 1) as usize;
                    max_val = max_val.max(image[ny * width + nx]);
                }
            }
            result[y * width + x] = max_val;
        }
    }
    result
}

/// Extract day and hour from in-game timestamp text.
/// Expects format like "Day 1234, 2056 Hours" -> "1234, 20:56".
fn extract_day_and_hour(text: &str) -> String {
    // Extract all digit/comma sequences
    let mut result = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() || c == ',' {
            result.push(c);
        }
    }

    // Remove first comma if exactly two commas (e.g., "1,234,2056" -> "1234,2056")
    if result.matches(',').count() == 2 {
        if let Some(idx) = result.find(',') {
            result.remove(idx);
        }
    }

    // Split by first comma and format time
    if let Some(comma_idx) = result.find(',') {
        let left = &result[..comma_idx];
        let right = &result[comma_idx + 1..];

        // Extract only digits from right side
        let digits: String = right.chars().filter(|c| c.is_ascii_digit()).collect();

        // Format as HH:MM if we have exactly 4 digits
        if digits.len() == 4 {
            return format!("{}, {}:{}", left, &digits[..2], &digits[2..]);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_region() {
        // Create a simple test image
        let mut image = vec![0u8; 10 * 10 * 3];
        // Set center pixel to red
        let center_idx = (5 * 10 + 5) * 3;
        image[center_idx] = 255;

        let region = extract_region(&image, 10, 10, 4, 4, 3, 3);

        // Region should be 3x3x3 = 27 bytes
        assert_eq!(region.len(), 27);

        // Check that the red pixel is captured
        let region_center = (1 * 3 + 1) * 3;
        assert_eq!(region[region_center], 255);
    }

    #[test]
    fn test_detect_crated_from_group() {
        use crate::detector::GroupInfo;

        let pipeline = ScanPipeline::new("db.h5", "tessdata", ScanConfig::default());
        let groups = vec![
            GroupInfo::new(3, 0), // First group: items 0-2
            GroupInfo::new(5, 3), // Second group: items 3-7
        ];

        // First group should not be crated
        assert!(!pipeline.detect_crated_from_group(0, &groups));
        assert!(!pipeline.detect_crated_from_group(2, &groups));

        // Second group should be crated
        assert!(pipeline.detect_crated_from_group(3, &groups));
        assert!(pipeline.detect_crated_from_group(7, &groups));
    }
}

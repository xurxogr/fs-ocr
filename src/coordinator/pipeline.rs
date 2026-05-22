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

use rayon::prelude::*;

use crate::config::ScanConfig;
use crate::detector::{BlackBoxDetector, DetectedRegions, GreyMaskDetector};
use crate::enums::ItemFaction;
use crate::enums::StockpileType;
use crate::error::{FsOcrError, Result};
use crate::image_utils;
use crate::models::{ItemCandidate, Stockpile, StockpileItem, Timing};
use crate::ocr::{digit_matcher, preprocess, TextExtractor};
use crate::template::database::TemplateDatabase;
use crate::template::matching::{MatchFilter, TemplateMatcher};
use crate::template::phash::compute_phash;

/// Main scanning pipeline for stockpile screenshots.
pub struct ScanPipeline {
    /// Template database path.
    database_path: String,
    /// OCR models directory path.
    data_path: String,
    /// Scan configuration.
    config: ScanConfig,
    /// Loaded template database (cached).
    database: Option<Arc<TemplateDatabase>>,
    /// Text extractor for shard/timestamp. Multilingual under ocr-full
    /// (the in-game timestamp line includes CJK/Cyrillic on those clients),
    /// otherwise ocrs (Latin-only).
    shard_extractor: Option<TextExtractor>,
    /// Text extractor for type/name (Tesseract if ocr-full, otherwise ocrs).
    text_extractor: Option<TextExtractor>,
}

impl ScanPipeline {
    /// Create a new scan pipeline.
    pub fn new<P: AsRef<Path>>(database_path: P, data_path: P, config: ScanConfig) -> Self {
        Self {
            database_path: database_path.as_ref().to_string_lossy().to_string(),
            data_path: data_path.as_ref().to_string_lossy().to_string(),
            config,
            database: None,
            shard_extractor: None,
            text_extractor: None,
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

        // Initialize shard extractor.
        // Shard names are Latin ("ABLE", "CHARLIE", "Devbranch"), but the
        // in-game timestamp line on CJK/Cyrillic clients embeds non-Latin
        // characters (e.g. "Day"/"Hours" localized), so use the multilingual
        // Tesseract extractor when available (ocr-full) and fall back to eng.
        if self.shard_extractor.is_none() {
            match TextExtractor::new_for_text_default("eng+chi_sim+rus") {
                Ok(extractor) => self.shard_extractor = Some(extractor),
                Err(_) => match TextExtractor::new_for_text_default("eng") {
                    Ok(extractor) => self.shard_extractor = Some(extractor),
                    Err(e) => {
                        eprintln!("Warning: Failed to initialize shard OCR: {}", e);
                    }
                },
            }
        }

        // Initialize text extractor for type/name
        // Uses Tesseract with multilingual support if ocr-full feature is enabled
        // Otherwise falls back to ocrs (Latin only)
        if self.text_extractor.is_none() {
            // Try multilingual first (only works with Tesseract/ocr-full)
            match TextExtractor::new_for_text_default("eng+chi_sim+rus") {
                Ok(extractor) => self.text_extractor = Some(extractor),
                Err(_) => {
                    // Fall back to English only
                    match TextExtractor::new_for_text_default("eng") {
                        Ok(extractor) => self.text_extractor = Some(extractor),
                        Err(e) => {
                            eprintln!("Warning: Failed to initialize text OCR: {}", e);
                        }
                    }
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

    /// Public wrapper for ensure_initialized (for debug methods).
    pub fn ensure_initialized_public(&mut self, resolution: i32) -> Result<()> {
        self.ensure_initialized(resolution)
    }

    /// Public wrapper for detect_stockpile_regions (for debug methods).
    pub fn detect_stockpile_regions_public(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
    ) -> Result<(crate::detector::DetectedRegions, f64, f64)> {
        self.detect_stockpile_regions(image, width, height)
    }

    /// Extract text from a region using English OCR (for debug).
    pub fn extract_text_from_region_public(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<String> {
        let extractor = match &self.text_extractor {
            Some(e) => e,
            None => return Ok("(OCR not initialized)".to_string()),
        };

        let scale_factor = height as f64 / 2160.0;
        let region_img = extract_region(
            image,
            width as usize,
            height as usize,
            x.max(0) as usize,
            y.max(0) as usize,
            w as usize,
            h as usize,
        );

        let (processed, proc_w, proc_h) =
            preprocess_light_text(&region_img, w as usize, h as usize, scale_factor, 2.0);

        let text = extractor.extract_text(&processed, proc_w as i32, proc_h as i32, 1)?;
        Ok(text)
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
        let mut timing = Timing::default();

        let detect_start = Instant::now();
        let (regions, blackbox_ms, greymask_ms) =
            match self.detect_stockpile_regions(image, width, height) {
                Ok(r) => r,
                Err(FsOcrError::NoStockpileDetected) => {
                    stockpile.add_error("No stockpile detected in image".to_string());
                    timing.detection_ms = Some(detect_start.elapsed().as_secs_f64() * 1000.0);
                    stockpile.timing = Some(timing);
                    return Ok(stockpile);
                }
                Err(e) => return Err(e),
            };
        timing.detection_ms = Some(detect_start.elapsed().as_secs_f64() * 1000.0);
        timing.blackbox_ms = Some(blackbox_ms);
        timing.greymask_ms = Some(greymask_ms);

        // Step 2: Extract quantities via OCR
        let quantity_start = Instant::now();
        let quantities = self.extract_quantities(image, width, height, &regions)?;
        timing.quantity_ms = Some(quantity_start.elapsed().as_secs_f64() * 1000.0);

        // Step 3: Match icons to templates
        let match_start = Instant::now();
        let items = self.match_icons(image, width, height, &regions, &quantities, faction)?;
        timing.matching_ms = Some(match_start.elapsed().as_secs_f64() * 1000.0);

        // Step 4: Extract stockpile metadata (type, name, shard)
        let metadata_start = Instant::now();
        self.extract_stockpile_metadata(image, width, height, &regions, &mut stockpile)?;
        timing.metadata_ms = Some(metadata_start.elapsed().as_secs_f64() * 1000.0);

        stockpile.timing = Some(timing);

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
        let scale_factor = regions.scale_factor;

        // Extract stockpile type (may need multilingual support for Chinese/Russian)
        if let Some((x, y, w, h)) = regions.type_region {
            if let Some(extractor) = &self.text_extractor {
                let type_img = extract_region(
                    image,
                    width as usize,
                    height as usize,
                    x.max(0) as usize,
                    y.max(0) as usize,
                    w as usize,
                    h as usize,
                );

                // Minimal preprocessing for OCR (no binarization for multilingual support)
                let (processed, proc_w, proc_h) =
                    preprocess_light_text(&type_img, w as usize, h as usize, scale_factor, 2.0);

                if let Ok(text) = extractor.extract_text(
                    &processed,
                    proc_w as i32,
                    proc_h as i32,
                    1, // grayscale
                ) {
                    let stockpile_type = StockpileType::from_string(&text);
                    stockpile.stockpile_type = stockpile_type;
                }
            }
        }

        // Extract shard and ingame timestamp.
        // Region contains 2 lines: timestamp on top, shard name on bottom
        // Split the region in half vertically and process each separately
        if let Some((x, y, w, h)) = regions.shard_region {
            if let Some(engine) = &self.shard_extractor {
                let half_h = h / 2;

                // Top half: timestamp line ("Day 702, 0304 Hours")
                let timestamp_img = extract_region(
                    image,
                    width as usize,
                    height as usize,
                    x.max(0) as usize,
                    y.max(0) as usize,
                    w as usize,
                    half_h as usize,
                );

                let (processed, proc_w, proc_h) =
                    preprocess_for_shard(&timestamp_img, w as usize, half_h as usize);

                if let Ok(text) = engine.extract_text(&processed, proc_w as i32, proc_h as i32, 1) {
                    let timestamp = extract_day_and_hour(&text);
                    if !timestamp.is_empty() {
                        stockpile.ingame_timestamp = Some(timestamp);
                    }
                }

                // Bottom half: shard name ("ABLE", "CHARLIE", "Devbranch")
                let shard_img = extract_region(
                    image,
                    width as usize,
                    height as usize,
                    x.max(0) as usize,
                    (y + half_h).max(0) as usize,
                    w as usize,
                    half_h as usize,
                );

                // Use higher upscaling for small shard text (4x instead of 2x)
                let (processed, proc_w, proc_h) =
                    preprocess_for_shard(&shard_img, w as usize, half_h as usize);

                if let Ok(text) = engine.extract_text(&processed, proc_w as i32, proc_h as i32, 1) {
                    let text_upper = text.to_uppercase();
                    if text_upper.contains("ABLE") {
                        stockpile.shard = Some("ABLE".to_string());
                    } else if text_upper.contains("CHARLIE") {
                        stockpile.shard = Some("CHARLIE".to_string());
                    } else if text_upper.contains("DEVBRANCH") || text_upper.contains("DEV") {
                        stockpile.shard = Some("Devbranch".to_string());
                    }
                }
            }
        }

        // Extract stockpile name (only for types that support custom names)
        // May need multilingual support for Chinese/Russian names
        if stockpile.stockpile_type.has_custom_name() {
            if let Some((x, y, w, h)) = regions.name_region {
                if let Some(extractor) = &self.text_extractor {
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
                        preprocess_light_text(&name_img, w as usize, h as usize, scale_factor, 4.0);

                    // Split into lines if multiline, OCR each, then join
                    let lines = split_text_lines(&processed, proc_w, proc_h);
                    let mut line_texts: Vec<String> = Vec::new();

                    for (line_img, line_w, line_h) in &lines {
                        if let Ok(text) = extractor.extract_text(
                            line_img,
                            *line_w as i32,
                            *line_h as i32,
                            1, // grayscale
                        ) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                line_texts.push(trimmed.to_string());
                            }
                        }
                    }

                    if !line_texts.is_empty() {
                        let name = join_multiline_name(&line_texts.join("\n"));
                        if !name.is_empty() {
                            stockpile.name = Some(name);
                        }
                    }
                }
            }

            stockpile.is_reserved = match &stockpile.name {
                None => false,
                Some(n) => {
                    let trimmed = n.trim();
                    !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("public")
                }
            };
        }

        Ok(())
    }

    /// Extract quantities using template-based digit matching.
    ///
    /// Primary method: template matching for Renner font digits.
    /// Fallback: OCR for failed recognitions (when ocr-full is enabled).
    fn extract_quantities(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        regions: &crate::detector::DetectedRegions,
    ) -> Result<Vec<i32>> {
        self.extract_quantities_template(image, width, height, regions)
    }

    /// Extract quantities using template-based digit matching (primary method).
    fn extract_quantities_template(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        regions: &crate::detector::DetectedRegions,
    ) -> Result<Vec<i32>> {
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

        let box_width = regions.box_width;
        let box_height = regions.box_height;
        let scale = regions.scale_factor;

        // Convert RGB image to grayscale for digit matching
        let grayscale = image_utils::rgb_to_grayscale(image, width as usize, height as usize);

        // Use template-based digit matching
        let quantities = digit_matcher::recognize_quantities_batch(
            &grayscale,
            width,
            height,
            &regions.quantity_boxes,
            box_width,
            box_height,
            scale,
        );

        Ok(quantities)
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
            self.config.ncc_initial_candidates,
            self.config.ncc_escalation_threshold,
        );

        // Step 2: Two-phase matching for better parallelization
        // Phase 1: Match first N items of each group sequentially (for category detection)
        // Phase 2: Match remaining items in parallel (with detected category)
        let total_items = icons_data.len();
        let mut items: Vec<StockpileItem> = vec![StockpileItem::unknown(-1, false); total_items];

        // Collect items for parallel processing and detected categories per group
        let mut parallel_items: Vec<(usize, Option<ItemCategory>)> = Vec::new();

        for (group_idx, group) in regions.groups.iter().enumerate() {
            let filter_start = if group_idx == 0 { 2 } else { 5 };
            let unfiltered_count = filter_start.min(group.size);

            let mut category_counts: HashMap<ItemCategory, usize> = HashMap::new();
            let mut detected_category: Option<ItemCategory> = None;

            // Phase 1: Sequential matching for category detection
            for i in 0..unfiltered_count.min(group.size) {
                let item_idx = group.start_index + i;
                if item_idx >= total_items {
                    break;
                }

                let (icon, phash, icon_w, icon_h) = &icons_data[item_idx];
                let quantity = quantities.get(item_idx).copied().unwrap_or(-1);

                let filter = MatchFilter::new().faction(faction);
                let result = matcher.match_icon_with_phash(icon, *icon_w, *icon_h, *phash, &filter);

                let item = match result {
                    Ok(match_result) if match_result.best_match.is_some() => {
                        let template = match_result.best_match.as_ref().unwrap();
                        *category_counts.entry(template.category).or_insert(0) += 1;

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

            // Detect category from first N items
            if !category_counts.is_empty() {
                detected_category = category_counts
                    .iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(cat, _)| *cat);
            }

            // Collect remaining items for parallel processing
            for i in unfiltered_count..group.size {
                let item_idx = group.start_index + i;
                if item_idx < total_items {
                    parallel_items.push((item_idx, detected_category));
                }
            }
        }

        // Phase 2: Parallel matching for remaining items
        let parallel_results: Vec<(usize, StockpileItem)> = parallel_items
            .par_iter()
            .map(|&(item_idx, category)| {
                let (icon, phash, icon_w, icon_h) = &icons_data[item_idx];
                let quantity = quantities.get(item_idx).copied().unwrap_or(-1);

                let filter = MatchFilter::new().faction(faction).category(category);
                let result = matcher.match_icon_with_phash(icon, *icon_w, *icon_h, *phash, &filter);

                let item = match result {
                    Ok(match_result) if match_result.best_match.is_some() => {
                        let template = match_result.best_match.as_ref().unwrap();

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
                        let crated =
                            item_idx >= regions.groups.first().map(|g| g.size).unwrap_or(0);
                        StockpileItem::unknown(quantity, crated)
                    }
                };

                (item_idx, item)
            })
            .collect();

        // Merge parallel results
        for (item_idx, item) in parallel_results {
            items[item_idx] = item;
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

    /// Get the data path (OCR models directory).
    pub fn data_path(&self) -> &str {
        &self.data_path
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

        // Step 4: Detect stockpile type/name regions based on info bar height
        if let Some(&(_, first_y)) = regions.quantity_boxes.first() {
            regions.info_bar_height = first_y - roi_y;
            detector.detect_stockpile_regions_with_info_bar(&mut regions, roi_x, roi_y);
        }

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

/// Split multiline text into separate line images.
/// Returns a vector of (image, width, height) tuples, one per line.
fn split_text_lines(image: &[u8], width: usize, height: usize) -> Vec<(Vec<u8>, usize, usize)> {
    // Find vertical bounds of text (bright pixels > 200)
    let mut min_y = height;
    let mut max_y = 0;
    for y in 0..height {
        for x in 0..width {
            if image[y * width + x] > 200 {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    let text_height = if max_y >= min_y { max_y - min_y + 1 } else { 0 };

    // If text doesn't fill most of height, return as single line
    if text_height < height * 60 / 100 {
        return vec![(image.to_vec(), width, height)];
    }

    // Find the gap between lines: row with fewest bright pixels in the middle region
    let search_start = min_y + text_height / 4;
    let search_end = min_y + text_height * 3 / 4;

    let mut min_bright = width + 1;
    let mut split_y = (search_start + search_end) / 2;

    for y in search_start..search_end {
        let bright_count = (0..width).filter(|&x| image[y * width + x] > 200).count();
        if bright_count < min_bright {
            min_bright = bright_count;
            split_y = y;
        }
    }

    // If no clear gap found (min_bright > 20% of width), return as single line
    if min_bright > width / 5 {
        return vec![(image.to_vec(), width, height)];
    }

    // Extract and tight-crop each line
    fn extract_tight_line(
        image: &[u8],
        width: usize,
        y_start: usize,
        y_end: usize,
    ) -> (Vec<u8>, usize, usize) {
        // Find actual text bounds within this line region
        let mut line_min_y = y_end;
        let mut line_max_y = y_start;
        let mut line_min_x = width;
        let mut line_max_x = 0;

        for y in y_start..y_end {
            for x in 0..width {
                if image[y * width + x] > 200 {
                    line_min_y = line_min_y.min(y);
                    line_max_y = line_max_y.max(y);
                    line_min_x = line_min_x.min(x);
                    line_max_x = line_max_x.max(x);
                }
            }
        }

        if line_max_y < line_min_y || line_max_x < line_min_x {
            // No text found, return empty
            return (vec![144u8; 1], 1, 1);
        }

        // Add small padding
        let pad = 2;
        let crop_x = line_min_x.saturating_sub(pad);
        let crop_y = line_min_y.saturating_sub(pad);
        let crop_w = (line_max_x - line_min_x + 1 + pad * 2).min(width - crop_x);
        let crop_h = (line_max_y - line_min_y + 1 + pad * 2).min(y_end - crop_y);

        let mut cropped = vec![144u8; crop_w * crop_h];
        for y in 0..crop_h {
            for x in 0..crop_w {
                cropped[y * crop_w + x] = image[(crop_y + y) * width + crop_x + x];
            }
        }

        (cropped, crop_w, crop_h)
    }

    let top_line = extract_tight_line(image, width, min_y, split_y);
    let bottom_line = extract_tight_line(image, width, split_y + 1, max_y + 1);

    vec![top_line, bottom_line]
}

/// Join multiline name text.
/// If a line ends with '-', concatenate directly; otherwise add a space.
fn join_multiline_name(text: &str) -> String {
    let mut result = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if result.is_empty() {
            result.push_str(trimmed);
        } else if result.ends_with('-') {
            result.push_str(trimmed);
        } else {
            result.push(' ');
            result.push_str(trimmed);
        }
    }
    result
}

/// Preprocess light text on dark background (type, name).
/// Uses luma.max(144) + tight crop around bright pixels + bilinear upscale.
///
/// Args:
///   upscale_base: Base upscale factor (2.0 for type, 4.0 for name)
fn preprocess_light_text(
    image: &[u8],
    width: usize,
    height: usize,
    scale_factor: f64,
    upscale_base: f64,
) -> (Vec<u8>, usize, usize) {
    // Convert RGB to grayscale with minimum brightness boost
    let mut grayscale = Vec::with_capacity(width * height);
    let mut raw_gray = Vec::with_capacity(width * height);
    for chunk in image.chunks_exact(3) {
        let luma =
            ((77u16 * chunk[0] as u16 + 150u16 * chunk[1] as u16 + 29u16 * chunk[2] as u16 + 128)
                >> 8) as u8;
        raw_gray.push(luma);
        grayscale.push(luma.max(144));
    }

    // Find tight bounds around bright pixels (luma > 200)
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;

    for y in 0..height {
        for x in 0..width {
            if raw_gray[y * width + x] > 200 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    // If no bright pixels found, use full image
    let upscale = upscale_base / scale_factor;
    if min_x > max_x || min_y > max_y {
        let new_w = ((width as f64) * upscale) as usize;
        let new_h = ((height as f64) * upscale) as usize;
        let upscaled = preprocess::upscale_bilinear(&grayscale, width, height, new_w, new_h);
        return (upscaled, new_w, new_h);
    }

    // Add padding around tight bounds
    let padding = height / 8;
    let crop_x1 = min_x.saturating_sub(padding);
    let crop_y1 = min_y.saturating_sub(padding);
    let crop_x2 = (max_x + 1 + padding).min(width);
    let crop_y2 = (max_y + 1 + padding).min(height);
    let crop_w = crop_x2 - crop_x1;
    let crop_h = crop_y2 - crop_y1;

    // Extract tight crop from boosted grayscale
    let mut cropped = Vec::with_capacity(crop_w * crop_h);
    for y in crop_y1..crop_y2 {
        for x in crop_x1..crop_x2 {
            cropped.push(grayscale[y * width + x]);
        }
    }

    // Upscale the tight crop
    let new_w = ((crop_w as f64) * upscale) as usize;
    let new_h = ((crop_h as f64) * upscale) as usize;
    let upscaled = preprocess::upscale_bilinear(&cropped, crop_w, crop_h, new_w, new_h);

    (upscaled, new_w, new_h)
}

/// Preprocess for shard/timestamp text.
/// Converts to grayscale with minimum brightness boost for better OCR.
fn preprocess_for_shard(image: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    let mut processed = Vec::with_capacity(width * height);

    for chunk in image.chunks_exact(3) {
        // Standard luma conversion: 0.299*R + 0.587*G + 0.114*B
        let luma =
            ((77u16 * chunk[0] as u16 + 150u16 * chunk[1] as u16 + 29u16 * chunk[2] as u16 + 128)
                >> 8) as u8;
        // Boost minimum brightness to help OCR recognize bright text on dark backgrounds
        processed.push(luma.max(144));
    }

    (processed, width, height)
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
    fn extracts_day_and_hour_from_plain_text() {
        assert_eq!(extract_day_and_hour("Day 1234, 2056 Hours"), "1234, 20:56");
        assert_eq!(extract_day_and_hour("Day 702, 0304 Hours"), "702, 03:04");
    }

    #[test]
    fn extracts_day_and_hour_from_cjk_and_cyrillic_text() {
        // CJK/Cyrillic clients surround the digits with non-Latin characters.
        // Only ASCII digits/commas are kept, so the result must still be clean.
        assert_eq!(extract_day_and_hour("第 702 天, 0304 小时"), "702, 03:04");
        assert_eq!(extract_day_and_hour("День 702, 0304 часов"), "702, 03:04");
    }

    #[test]
    fn test_detect_crated_from_group() {
        use crate::detector::GroupInfo;

        let pipeline = ScanPipeline::new("db.h5", "data", ScanConfig::default());
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

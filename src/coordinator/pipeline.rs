//! Main scanning pipeline orchestration.
//!
//! Coordinates the detection, OCR, and template matching stages
//! to produce a complete stockpile scan result.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[cfg(not(feature = "ocr-full"))]
use std::collections::HashMap;
#[cfg(not(feature = "ocr-full"))]
use std::sync::Mutex;

/// Maximum total boxes to process (prevents DoS via excessive memory allocation).
/// A typical Foxhole stockpile has 6 columns × ~10 rows = 60 items max per view.
const MAX_TOTAL_BOXES: usize = 200;

use rayon::prelude::*;

use super::debug_ocr;
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

/// Decode-time character masks for the ocrs backend (see `OcrConfig::allowed_chars`).
/// Restricting the recognizer to a field's plausible character set keeps
/// closed-vocabulary reads on-script (e.g. a Latin shard never decodes to
/// Cyrillic) and stops marker words from being hallucinated as stray digits.
///
/// Shard names are a fixed Latin set (ABLE / CHARLIE / DevBranch / LIVE).
#[cfg(not(feature = "ocr-full"))]
const SHARD_MASK: &str = "ABCDEHILRVacehnrv";

/// Timestamp masks are per script, matching the 3-way `ClientLanguage`. Each
/// allows digits, spaces and the separators (`, . : -`) plus that script's
/// letters, so the localized `Day`/`Hours` markers decode as letters (which the
/// parser discards) instead of corrupting the digit run. Latin uses the full
/// alphabet so any Latin client (EN/DE/FR/PT/…) reads correctly.
#[cfg(not(feature = "ocr-full"))]
const TIME_MASK_LATIN: &str = "0123456789 ,.:-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
#[cfg(not(feature = "ocr-full"))]
const TIME_MASK_CYRILLIC: &str =
    "0123456789 ,.:-АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя";
#[cfg(not(feature = "ocr-full"))]
const TIME_MASK_CHINESE: &str = "0123456789,，日时分";

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
    /// Multilingual extractor for the shard/timestamp block on CJK/Cyrillic
    /// clients. Reads the whole region (timestamp line + shard line) as one
    /// block; the localized timestamp embeds CJK/Cyrillic, so it needs the full
    /// `eng+chi_sim+rus` model. Routed to only when the client language detected
    /// from the type region is Chinese or Russian.
    block_extractor_multi: Option<TextExtractor>,
    /// English-only extractor for the shard/timestamp block on Latin clients.
    /// When the type region reads as non-CJK/non-Cyrillic the whole block is
    /// Latin, so eng-only avoids the multilingual model misreading Latin text
    /// (e.g. "ABLE" -> Cyrillic "АВЕЕ").
    block_extractor_eng: Option<TextExtractor>,
    /// Text extractor for type/name (Tesseract if ocr-full, otherwise ocrs).
    text_extractor: Option<TextExtractor>,
    /// Lazily-built ocrs extractors keyed by their `allowed_chars` mask. A scan
    /// touches at most two masks (the fixed shard mask plus one script's
    /// timestamp mask), so this caches them across scans without reloading the
    /// recognition model more than once per distinct mask. ocrs-only — the
    /// Tesseract backend reads the region as one block and does not use masks.
    #[cfg(not(feature = "ocr-full"))]
    masked_extractors: Mutex<HashMap<String, TextExtractor>>,
}

impl ScanPipeline {
    /// Create a new scan pipeline.
    pub fn new<P: AsRef<Path>>(database_path: P, data_path: P, config: ScanConfig) -> Self {
        Self {
            database_path: database_path.as_ref().to_string_lossy().to_string(),
            data_path: data_path.as_ref().to_string_lossy().to_string(),
            config,
            database: None,
            block_extractor_multi: None,
            block_extractor_eng: None,
            text_extractor: None,
            #[cfg(not(feature = "ocr-full"))]
            masked_extractors: Mutex::new(HashMap::new()),
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

        // Initialize the multilingual block extractor.
        // The shard/timestamp region is read as one block. On CJK/Cyrillic
        // clients the in-game timestamp line embeds non-Latin characters
        // (e.g. localized "Day"/"Hours") that surround the digits and affect
        // segmentation, so it MUST use the multilingual Tesseract model
        // (ocr-full) and fall back to eng. Do NOT make this English-only — it
        // has been tested; dropping the non-Latin langs breaks the number reads.
        if self.block_extractor_multi.is_none() {
            match TextExtractor::new_for_text_block_default("eng+chi_sim+rus") {
                Ok(extractor) => self.block_extractor_multi = Some(extractor),
                Err(_) => match TextExtractor::new_for_text_block_default("eng") {
                    Ok(extractor) => self.block_extractor_multi = Some(extractor),
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to initialize multilingual block OCR: {}",
                            e
                        );
                    }
                },
            }
        }

        // Initialize the English-only block extractor.
        // When the client language detected from the type region is neither
        // Chinese nor Russian, the whole shard/timestamp block is Latin. The
        // multilingual model misreads small Latin crops as Cyrillic
        // (e.g. "DevBranch" -> "Беубгапсп"), so Latin clients read the block
        // with this eng-only extractor instead.
        if self.block_extractor_eng.is_none() {
            match TextExtractor::new_for_text_block_default("eng") {
                Ok(extractor) => self.block_extractor_eng = Some(extractor),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize English block OCR: {}", e);
                }
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

    /// Eagerly build the masked recognition engines so the first scan of each
    /// script doesn't pay the model-load cost. Long-lived (library) callers can
    /// call this once up front; CLI single-shot callers can skip it and let the
    /// engines build lazily on first use. Builds the fixed shard mask plus the
    /// timestamp masks for every `ClientLanguage` (Latin/Cyrillic/Chinese), so
    /// no argument is needed — there are only three scripts. No-op on the
    /// Tesseract (`ocr-full`) backend, which doesn't use decode masks.
    #[cfg(not(feature = "ocr-full"))]
    pub fn warmup(&self) -> Result<()> {
        let mut cache = self
            .masked_extractors
            .lock()
            .map_err(|e| FsOcrError::Ocr(format!("Masked extractor lock poisoned: {}", e)))?;
        let masks = [
            SHARD_MASK,
            ClientLanguage::English.time_mask(),
            ClientLanguage::Russian.time_mask(),
            ClientLanguage::Chinese.time_mask(),
        ];
        for mask in masks {
            cache.entry(mask.to_string()).or_insert_with(|| {
                TextExtractor::new_for_text_default_with_allowed("eng", mask).unwrap_or_default()
            });
        }
        Ok(())
    }

    /// No-op warmup on the Tesseract backend (no decode masks to build).
    #[cfg(feature = "ocr-full")]
    pub fn warmup(&self) -> Result<()> {
        Ok(())
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
    #[allow(clippy::too_many_arguments)]
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

        // Optional: dump an annotated overlay of every detected region.
        if debug_ocr::enabled() {
            debug_ocr::save_regions_overlay(image, width as usize, height as usize, &regions);
        }

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

        // Extract stockpile type (may need multilingual support for Chinese/Russian).
        // The type text also tells us the client language, which we use below to
        // route the shard/timestamp block to the right OCR model.
        let mut client_language = ClientLanguage::English;
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

                if debug_ocr::enabled() {
                    debug_ocr::save_gray("type", &processed, proc_w, proc_h);
                }

                if let Ok(text) = extractor.extract_text(
                    &processed,
                    proc_w as i32,
                    proc_h as i32,
                    1, // grayscale
                ) {
                    if debug_ocr::enabled() {
                        eprintln!("[FS_DEBUG_OCR] type region raw text: {:?}", text);
                    }
                    client_language = ClientLanguage::detect(&text);
                    let stockpile_type = StockpileType::from_string(&text);
                    stockpile.stockpile_type = stockpile_type;
                }
            }
        }

        // Extract shard and ingame timestamp. The region holds 2 lines:
        // timestamp on top, shard name on bottom.
        //
        // The OCR model is picked by the client language detected from the type
        // above: Latin clients use the eng-only extractor (the multilingual model
        // misreads small Latin text as Cyrillic), CJK/Cyrillic clients use the
        // multilingual one (their timestamp line embeds non-Latin glyphs).
        if let Some((x, y, w, h)) = regions.shard_region {
            let engine = match client_language {
                ClientLanguage::English => self
                    .block_extractor_eng
                    .as_ref()
                    .or(self.block_extractor_multi.as_ref()),
                ClientLanguage::Chinese | ClientLanguage::Russian => self
                    .block_extractor_multi
                    .as_ref()
                    .or(self.block_extractor_eng.as_ref()),
            };

            if let Some(engine) = engine {
                self.read_shard_region(
                    image,
                    width,
                    height,
                    (x, y, w, h),
                    engine,
                    client_language,
                    stockpile,
                );
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

                    // The game wraps long names across two rows. Detect genuine
                    // row wrapping (a tall blank gap between text bands) and, when
                    // present, lay the rows side by side into a single logical line
                    // before OCR. This both reconstructs the original name and lets
                    // Tesseract use line context — isolated single glyphs (e.g. a
                    // lone CJK character per row) are otherwise misread.
                    let lines = split_text_lines(&processed, proc_w, proc_h);

                    if debug_ocr::enabled() {
                        if lines.len() > 1 {
                            for (i, (buf, lw, lh)) in lines.iter().enumerate() {
                                debug_ocr::save_gray(&format!("name_line{i}"), buf, *lw, *lh);
                            }
                        } else {
                            debug_ocr::save_gray("name", &processed, proc_w, proc_h);
                        }
                    }

                    let (ocr_img, ocr_w, ocr_h) = if lines.len() > 1 {
                        join_lines_horizontally(&lines)
                    } else {
                        (processed, proc_w, proc_h)
                    };

                    if debug_ocr::enabled() && lines.len() > 1 {
                        debug_ocr::save_gray("name_merged", &ocr_img, ocr_w, ocr_h);
                    }

                    if let Ok(text) =
                        extractor.extract_text(&ocr_img, ocr_w as i32, ocr_h as i32, 1)
                    {
                        let name = text.trim();
                        if !name.is_empty() {
                            stockpile.name = Some(name.to_string());
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

    /// Read the timestamp and shard name from the shard region.
    ///
    /// The reading strategy differs by OCR backend (selected at compile time):
    ///
    /// - **Tesseract (`ocr-full`)**: read the whole region as one block. PSM 6
    ///   segments the two lines internally, and on CJK/Cyrillic clients this is
    ///   what reads the localized timestamp correctly — splitting it first
    ///   regressed those reads, so we deliberately do not split here.
    /// - **ocrs (default)**: ocrs recognizes a single rect with no line
    ///   detection, so a 2-line crop collapses into garbage. We split the region
    ///   into its top (timestamp) and bottom (shard) halves and read each as a
    ///   single line. ocrs is Latin-only, so CJK/Cyrillic clients are out of
    ///   scope for this backend regardless.
    #[cfg(feature = "ocr-full")]
    fn read_shard_region(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        region: (i32, i32, i32, i32),
        engine: &TextExtractor,
        _client_language: ClientLanguage,
        stockpile: &mut Stockpile,
    ) {
        let (x, y, w, h) = region;
        let block_img = extract_region(
            image,
            width as usize,
            height as usize,
            x.max(0) as usize,
            y.max(0) as usize,
            w as usize,
            h as usize,
        );

        // The region is two text lines; upscale toward a legible per-line height
        // so low-res crops stay readable as a block.
        let (processed, proc_w, proc_h) =
            preprocess_for_shard(&block_img, w as usize, h as usize, 2);

        if debug_ocr::enabled() {
            debug_ocr::save_gray("shard_block", &processed, proc_w, proc_h);
        }

        if let Ok(text) = engine.extract_text(&processed, proc_w as i32, proc_h as i32, 1) {
            // PSM 6 reads the block as multiple lines. Classify each: the line
            // carrying the digits is the timestamp, the line matching a known
            // shard is the shard name.
            for line in text.lines() {
                let timestamp = extract_day_and_hour(line);
                if !timestamp.is_empty() {
                    stockpile.ingame_timestamp = Some(timestamp);
                    break;
                }
            }
            for line in text.lines() {
                if let Some(shard) = match_shard_name(line) {
                    stockpile.shard = Some(shard.to_string());
                    break;
                }
            }
        }
    }

    /// Run the ocrs recognizer over `image` with a decode mask, building and
    /// caching one masked extractor per distinct `allowed_chars` string. The
    /// cache lock is held across the recognition call, which is fine: ocrs runs
    /// single-threaded (RTEN_NUM_THREADS=1) and scans are serial.
    #[cfg(not(feature = "ocr-full"))]
    fn extract_with_mask(
        &self,
        allowed_chars: &str,
        image: &[u8],
        width: i32,
        height: i32,
    ) -> Result<String> {
        let mut cache = self
            .masked_extractors
            .lock()
            .map_err(|e| FsOcrError::Ocr(format!("Masked extractor lock poisoned: {}", e)))?;
        let extractor = cache.entry(allowed_chars.to_string()).or_insert_with(|| {
            TextExtractor::new_for_text_default_with_allowed("eng", allowed_chars)
                .unwrap_or_default()
        });
        extractor.extract_text(image, width, height, 1)
    }

    /// See the `ocr-full` variant above for the rationale behind the per-backend
    /// split. This (ocrs) path reads the timestamp and shard as two single-line
    /// half-crops, each with a decode mask: the timestamp uses the client's
    /// script mask, the shard the fixed Latin shard mask.
    #[cfg(not(feature = "ocr-full"))]
    fn read_shard_region(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        region: (i32, i32, i32, i32),
        _engine: &TextExtractor,
        client_language: ClientLanguage,
        stockpile: &mut Stockpile,
    ) {
        let (x, y, w, h) = region;
        let half_h = h / 2;

        // Top half: timestamp line ("Day 702, 0304 Hours").
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
            preprocess_for_shard(&timestamp_img, w as usize, half_h as usize, 1);

        if debug_ocr::enabled() {
            debug_ocr::save_gray("timestamp", &processed, proc_w, proc_h);
        }

        if let Ok(text) = self.extract_with_mask(
            client_language.time_mask(),
            &processed,
            proc_w as i32,
            proc_h as i32,
        ) {
            if debug_ocr::enabled() {
                eprintln!("[FS_DEBUG_OCR] timestamp region raw text: {:?}", text);
            }
            let timestamp = extract_day_and_hour(&text);
            if !timestamp.is_empty() {
                stockpile.ingame_timestamp = Some(timestamp);
            }
        }

        // Bottom half: shard name ("ABLE", "CHARLIE", "Devbranch").
        let shard_img = extract_region(
            image,
            width as usize,
            height as usize,
            x.max(0) as usize,
            (y + half_h).max(0) as usize,
            w as usize,
            half_h as usize,
        );
        let (processed, proc_w, proc_h) =
            preprocess_for_shard(&shard_img, w as usize, half_h as usize, 1);

        if debug_ocr::enabled() {
            debug_ocr::save_gray("shard", &processed, proc_w, proc_h);
        }

        if let Ok(text) =
            self.extract_with_mask(SHARD_MASK, &processed, proc_w as i32, proc_h as i32)
        {
            if debug_ocr::enabled() {
                eprintln!("[FS_DEBUG_OCR] shard region raw text: {:?}", text);
            }
            if let Some(shard) = match_shard_name(&text) {
                stockpile.shard = Some(shard.to_string());
            }
        }
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

/// Split a name buffer into separate row images when the game has wrapped the
/// name across multiple rows.
///
/// Returns one (image, width, height) tuple per detected text row. A single row
/// is returned unchanged (the whole buffer). Rows are only split apart on a
/// genuine blank gap — a run of consecutive rows with no text pixels that is
/// tall relative to the text itself — so the internal horizontal gaps inside a
/// glyph (e.g. between the strokes of a CJK character) never cause a false
/// split, and a normal single line is never cut through its x-height.
fn split_text_lines(image: &[u8], width: usize, height: usize) -> Vec<(Vec<u8>, usize, usize)> {
    let bands = detect_text_bands(image, width, height);

    // 0 or 1 band: not a wrapped name — return the whole buffer untouched so the
    // OCR engine sees the line with its original surrounding margin.
    if bands.len() <= 1 {
        return vec![(image.to_vec(), width, height)];
    }

    bands
        .iter()
        .map(|&(y_start, y_end)| extract_tight_line(image, width, y_start, y_end))
        .collect()
}

/// Find vertical text bands separated by genuine blank gaps.
///
/// A row is "text" if it contains any bright pixel (> 200; text is bright on a
/// dark background after autocontrast). Contiguous text rows form a raw band;
/// raw bands separated by a blank run shorter than `gap_min` are merged so that
/// intra-glyph gaps stay within a single band. `gap_min` scales with the
/// tallest band so the threshold adapts to the rendered text size.
fn detect_text_bands(image: &[u8], width: usize, height: usize) -> Vec<(usize, usize)> {
    let row_has_text: Vec<bool> = (0..height)
        .map(|y| (0..width).any(|x| image[y * width + x] > 200))
        .collect();

    let mut raw: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (y, &has_text) in row_has_text.iter().enumerate() {
        match (has_text, start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                raw.push((s, y));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        raw.push((s, height));
    }

    if raw.is_empty() {
        return raw;
    }

    let tallest = raw.iter().map(|&(s, e)| e - s).max().unwrap_or(0);
    let gap_min = (tallest / 6).max(8);

    let mut merged: Vec<(usize, usize)> = vec![raw[0]];
    for &(s, e) in &raw[1..] {
        let last = merged.last_mut().unwrap();
        if s - last.1 < gap_min {
            last.1 = e;
        } else {
            merged.push((s, e));
        }
    }
    merged
}

/// Tight-crop the text within a row band, with a small padding margin.
fn extract_tight_line(
    image: &[u8],
    width: usize,
    y_start: usize,
    y_end: usize,
) -> (Vec<u8>, usize, usize) {
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
        return (vec![0u8; 1], 1, 1);
    }

    let pad = 2;
    let crop_x = line_min_x.saturating_sub(pad);
    let crop_y = line_min_y.saturating_sub(pad);
    let crop_w = (line_max_x - line_min_x + 1 + pad * 2).min(width - crop_x);
    let crop_h = (line_max_y - line_min_y + 1 + pad * 2).min(y_end - crop_y);

    let mut cropped = vec![0u8; crop_w * crop_h];
    for y in 0..crop_h {
        for x in 0..crop_w {
            cropped[y * crop_w + x] = image[(crop_y + y) * width + crop_x + x];
        }
    }

    (cropped, crop_w, crop_h)
}

/// Lay wrapped name rows side by side into one logical line for OCR.
///
/// The game wraps a single name across rows; reassembling them horizontally
/// reconstructs the original line so the OCR engine reads it with proper line
/// context (a lone glyph per row is otherwise misread). Rows are placed
/// left-to-right on a dark canvas with a small inter-row gap and a quiet-zone
/// border, each vertically centred.
fn join_lines_horizontally(lines: &[(Vec<u8>, usize, usize)]) -> (Vec<u8>, usize, usize) {
    const GAP: usize = 16;
    const PAD: usize = 30;

    let inner_h = lines.iter().map(|&(_, _, h)| h).max().unwrap_or(0);
    let inner_w: usize =
        lines.iter().map(|&(_, w, _)| w).sum::<usize>() + GAP * lines.len().saturating_sub(1);

    let canvas_w = inner_w + 2 * PAD;
    let canvas_h = inner_h + 2 * PAD;
    let mut canvas = vec![0u8; canvas_w * canvas_h];

    let mut x_off = PAD;
    for (line, lw, lh) in lines {
        let y_off = PAD + (inner_h - lh) / 2;
        for y in 0..*lh {
            for x in 0..*lw {
                canvas[(y_off + y) * canvas_w + (x_off + x)] = line[y * lw + x];
            }
        }
        x_off += lw + GAP;
    }

    (canvas, canvas_w, canvas_h)
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
    // Convert RGB to grayscale.
    let mut grayscale = Vec::with_capacity(width * height);
    for chunk in image.chunks_exact(3) {
        let luma =
            ((77u16 * chunk[0] as u16 + 150u16 * chunk[1] as u16 + 29u16 * chunk[2] as u16 + 128)
                >> 8) as u8;
        grayscale.push(luma);
    }

    // Stretch contrast so text becomes legible regardless of base brightness.
    // The previous approach floored brightness and cropped around near-white
    // pixels, which only worked for bright text on a dark info bar. Localized
    // names like the Chinese "公共" are low-contrast grey-on-grey and were
    // washed out; autocontrast normalizes both cases to full dynamic range.
    autocontrast(&mut grayscale, 2);

    // Trim blank left/right margins down to the text extent before upscaling.
    // The detection box is padded, so names/types carry a wide blank margin that
    // the recognizer otherwise reads as a spurious leading glyph (e.g. a stray
    // "8" before "ABDC-AST-B"). A margin of half the line height keeps breathing
    // room. Trimming pre-upscale also avoids enlarging dead pixels.
    let margin = height / 2;
    let (grayscale, width) = trim_columns_to_content(&grayscale, width, height, margin);

    // Upscale for better OCR.
    let upscale = upscale_base / scale_factor;
    let new_w = ((width as f64) * upscale) as usize;
    let new_h = ((height as f64) * upscale) as usize;
    let upscaled = preprocess::upscale_bilinear(&grayscale, width, height, new_w, new_h);

    (upscaled, new_w, new_h)
}

/// Stretch the grayscale histogram to full [0, 255] range in place.
///
/// Mirrors PIL's `ImageOps.autocontrast`: `cutoff_percent` of the pixel
/// population is clipped from each end of the histogram before computing the
/// low/high bounds, so a few outlier pixels don't dominate the mapping.
/// Crop blank left/right margins of a single-line grayscale strip down to the
/// text extent (plus `margin` columns of breathing room on each side).
///
/// A column carrying text spans both ink and background pixels vertically, so
/// its luma range is large; a blank column is near-uniform. Using the per-column
/// range makes this polarity-agnostic, which matters because the UI theme can
/// render the strip as light-on-dark or dark-on-light. If no column clears the
/// activity threshold (a genuinely blank strip), the input is returned unchanged
/// rather than cropped to nothing.
fn trim_columns_to_content(
    gray: &[u8],
    width: usize,
    height: usize,
    margin: usize,
) -> (Vec<u8>, usize) {
    if width == 0 || height == 0 {
        return (gray.to_vec(), width);
    }

    // 64 ≈ a quarter of the full 0..=255 range that autocontrast stretches to.
    const ACTIVITY_THRESHOLD: u8 = 64;

    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for x in 0..width {
        let (mut min, mut max) = (255u8, 0u8);
        for y in 0..height {
            let v = gray[y * width + x];
            min = min.min(v);
            max = max.max(v);
        }
        if max - min >= ACTIVITY_THRESHOLD {
            first.get_or_insert(x);
            last = x;
        }
    }

    let Some(first) = first else {
        return (gray.to_vec(), width);
    };

    let lo = first.saturating_sub(margin);
    let hi = (last + margin + 1).min(width);
    let new_w = hi - lo;

    let mut out = Vec::with_capacity(new_w * height);
    for y in 0..height {
        let row = y * width;
        out.extend_from_slice(&gray[row + lo..row + hi]);
    }
    (out, new_w)
}

/// Pad dark rows above and below the text so the bright text band occupies
/// roughly `target_fraction` of the frame height.
///
/// The recognizer is trained on lines rendered at font sizes 28-48 inside a
/// 64px frame, so glyphs fill ~45-75% of the height. An in-game timestamp/shard
/// crop trims tight to the text (~80-100% of the strip height); once the
/// recognizer resizes it to 64px the glyphs are proportionally larger than
/// anything in the training set, which tips CJK markers like `分` into digit
/// misreads (observed: `02时47分` decoded as `02时474`). Re-padding the vertical
/// frame toward the training ratio brings the input back in-distribution.
///
/// Only ever pads (never crops): a no-op when the band already sits within the
/// target fraction.
fn pad_rows_to_text_fraction(
    gray: &[u8],
    width: usize,
    height: usize,
    target_fraction: f64,
) -> (Vec<u8>, usize) {
    if width == 0 || height == 0 {
        return (gray.to_vec(), height);
    }

    // Same activity threshold as the column trim: ~a quarter of the autocontrast
    // range marks a row as carrying text.
    const TEXT_THRESHOLD: u8 = 64;

    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for y in 0..height {
        let row = &gray[y * width..y * width + width];
        if row.iter().copied().max().unwrap_or(0) >= TEXT_THRESHOLD {
            first.get_or_insert(y);
            last = y;
        }
    }

    let Some(first) = first else {
        return (gray.to_vec(), height);
    };

    let band = last - first + 1;
    let desired = (band as f64 / target_fraction).round() as usize;
    if desired <= height {
        return (gray.to_vec(), height);
    }

    let pad = desired - height;
    let top = pad / 2;
    let bottom = pad - top;

    let mut out = vec![0u8; width * top];
    out.extend_from_slice(gray);
    out.resize(out.len() + width * bottom, 0u8);
    (out, desired)
}

fn autocontrast(gray: &mut [u8], cutoff_percent: u32) {
    if gray.is_empty() {
        return;
    }

    let mut hist = [0u32; 256];
    for &v in gray.iter() {
        hist[v as usize] += 1;
    }

    let cut = (gray.len() as u32 * cutoff_percent) / 100;

    // Lowest value with population remaining after clipping `cut` from the bottom.
    let mut acc = 0u32;
    let mut lo = 0usize;
    for (v, &count) in hist.iter().enumerate() {
        acc += count;
        if acc > cut {
            lo = v;
            break;
        }
    }

    // Highest value with population remaining after clipping `cut` from the top.
    let mut acc = 0u32;
    let mut hi = 255usize;
    for (v, &count) in hist.iter().enumerate().rev() {
        acc += count;
        if acc > cut {
            hi = v;
            break;
        }
    }

    if hi <= lo {
        return; // Flat or inverted range — nothing to stretch.
    }

    let span = (hi - lo) as f32;
    for v in gray.iter_mut() {
        let clamped = (*v as usize).clamp(lo, hi);
        *v = (((clamped - lo) as f32 / span) * 255.0).round() as u8;
    }
}

/// Preprocess for shard/timestamp text.
///
/// Converts to grayscale and stretches the histogram to full range. The earlier
/// `luma.max(144)` brightness floor assumed bright text on a dark bar; on the
/// dark UI theme the text is dark on a dark panel, so flooring collapsed the
/// whole region to a flat grey and erased the text. Autocontrast normalizes
/// both themes to full dynamic range while preserving polarity.
fn preprocess_for_shard(
    image: &[u8],
    width: usize,
    height: usize,
    lines: usize,
) -> (Vec<u8>, usize, usize) {
    let mut processed = Vec::with_capacity(width * height);

    for chunk in image.chunks_exact(3) {
        // Standard luma conversion: 0.299*R + 0.587*G + 0.114*B
        let luma =
            ((77u16 * chunk[0] as u16 + 150u16 * chunk[1] as u16 + 29u16 * chunk[2] as u16 + 128)
                >> 8) as u8;
        processed.push(luma);
    }

    autocontrast(&mut processed, 2);

    // Normalize polarity to light-on-dark. The recognizer is trained on bright
    // text over a dark background (matching the in-game type banner), but the
    // shard/timestamp strip can render dark-on-light depending on the UI theme;
    // fed inverted, the model decodes to junk. After autocontrast the background
    // dominates one extreme, so a bright mean means dark-text-on-light and we
    // flip it.
    let mean: u32 =
        processed.iter().map(|&v| v as u32).sum::<u32>() / processed.len().max(1) as u32;
    if mean > 127 {
        for v in processed.iter_mut() {
            *v = 255 - *v;
        }
    }

    // Crop the blank left/right margins down to the text extent. The strips
    // place a short word (e.g. "ABLE") in a wide region, leaving most of the
    // crop empty; the recognizer expects a line image where text fills the
    // frame, and a mostly-empty strip decodes to junk. Margin is half a line
    // height so the glyphs keep a little breathing room.
    let margin = (height / lines.max(1)) / 2;
    let (processed, width) = trim_columns_to_content(&processed, width, height, margin);

    // Re-pad the vertical frame toward the recognizer's training text-to-frame
    // ratio. Only meaningful for the single-line crops (timestamp, shard); the
    // multi-line block path stacks rows and would need per-row banding, so leave
    // it untouched there.
    let (processed, height) = if lines == 1 {
        pad_rows_to_text_fraction(&processed, width, height, 0.62)
    } else {
        (processed, height)
    };

    // Upscale tiny regions toward a legible line height. At low resolutions a
    // single shard/timestamp line can be ~13px tall, below what Tesseract reads
    // reliably; scaling it up recovers the text. The factor targets a per-line
    // height (the region stacks `lines` text rows) rather than blindly
    // multiplying — over-upscaling blurs and hurts OCR.
    const TARGET_LINE_HEIGHT: usize = 26;
    let line_height = (height / lines.max(1)).max(1);
    let factor = ((TARGET_LINE_HEIGHT + line_height / 2) / line_height).max(1);
    if factor > 1 {
        let new_w = width * factor;
        let new_h = height * factor;
        let upscaled = preprocess::upscale_bilinear(&processed, width, height, new_w, new_h);
        return (upscaled, new_w, new_h);
    }

    (processed, width, height)
}

/// Client UI language, inferred from the stockpile type text.
///
/// Used to route the shard/timestamp block to the right OCR model: Latin
/// clients read it with an eng-only model, CJK/Cyrillic clients with the
/// multilingual one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientLanguage {
    English,
    Chinese,
    Russian,
}

impl ClientLanguage {
    /// Detect the client language from OCR'd type text by script.
    ///
    /// CJK ideographs imply the Chinese client; Cyrillic implies the Russian
    /// client; anything else is treated as a Latin (English) client.
    fn detect(text: &str) -> Self {
        let has_cjk = text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
        if has_cjk {
            return ClientLanguage::Chinese;
        }
        let has_cyrillic = text.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c));
        if has_cyrillic {
            return ClientLanguage::Russian;
        }
        ClientLanguage::English
    }

    /// The timestamp decode mask for this client's script.
    #[cfg(not(feature = "ocr-full"))]
    fn time_mask(self) -> &'static str {
        match self {
            ClientLanguage::English => TIME_MASK_LATIN,
            ClientLanguage::Russian => TIME_MASK_CYRILLIC,
            ClientLanguage::Chinese => TIME_MASK_CHINESE,
        }
    }
}

/// Known shard names. The shard-name crop is a single Latin word.
const KNOWN_SHARDS: [&str; 4] = ["ABLE", "CHARLIE", "LIVE", "Devbranch"];

/// Match OCR'd shard text to the closest known shard name.
///
/// At low resolutions OCR garbles a character or two (e.g. "Devbranch" reads as
/// "Vevoranch" when the `D`'s stem blurs), so exact substring matching fails.
/// We instead pick the known shard with the highest character similarity and
/// accept it only above a confidence threshold — close enough to absorb a couple
/// of misread glyphs, strict enough to reject unrelated text.
fn match_shard_name(text: &str) -> Option<&'static str> {
    const MIN_SIMILARITY: f64 = 0.6;

    let candidate = text.trim().to_lowercase();
    if candidate.is_empty() {
        return None;
    }

    KNOWN_SHARDS
        .iter()
        .map(|&shard| {
            (
                shard,
                crate::text_utils::similarity(&candidate, &shard.to_lowercase()),
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|&(_, similarity)| similarity >= MIN_SIMILARITY)
        .map(|(shard, _)| shard)
}

/// Extract day and hour from in-game timestamp text.
/// Expects format like "Day 1234, 2056 Hours" -> "1234, 20:56".
///
/// The day/time separator is unreliable across locales: the English client
/// emits an ASCII comma, the Chinese client a fullwidth comma (`，`), and OCR
/// sometimes drops it entirely. So we don't depend on it — the time is always
/// the trailing 4 digits (HHMM) and the day is whatever precedes them.
fn extract_day_and_hour(text: &str) -> String {
    let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();

    // Need at least the 4 time digits plus 1 day digit. Fewer than 5 means we
    // can't tell the day from the time, so treat it as noise.
    if digits.len() < 5 {
        return String::new();
    }

    let split = digits.len() - 4;
    let day = &digits[..split];
    let hhmm = &digits[split..];
    format!("{}, {}:{}", day, &hhmm[..2], &hhmm[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_shard_name_matches_exact_names() {
        assert_eq!(match_shard_name("ABLE"), Some("ABLE"));
        assert_eq!(match_shard_name("CHARLIE"), Some("CHARLIE"));
        assert_eq!(match_shard_name("LIVE"), Some("LIVE"));
        assert_eq!(match_shard_name("Devbranch"), Some("Devbranch"));
    }

    #[test]
    fn match_shard_name_tolerates_low_res_misreads() {
        // Observed eng OCR on a 1600x900 crop: the blurred `D` reads as `V`.
        assert_eq!(match_shard_name("Vevoranch"), Some("Devbranch"));
        assert_eq!(match_shard_name("DevBranch"), Some("Devbranch"));
    }

    #[test]
    fn match_shard_name_rejects_unrelated_and_empty() {
        assert_eq!(match_shard_name(""), None);
        assert_eq!(match_shard_name("   "), None);
        assert_eq!(match_shard_name("Public"), None);
    }

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
        // Real in-game formats: Chinese uses a fullwidth comma (`，`, U+FF0C)
        // and the 日/时/分 markers; the day carries a thousands separator. The
        // non-digit glyphs and separators are all stripped before parsing.
        assert_eq!(extract_day_and_hour("1,529日，08时51分"), "1529, 08:51");
        assert_eq!(
            extract_day_and_hour("1,529-й день, 08:51 часов"),
            "1529, 08:51"
        );
    }

    #[test]
    fn extracts_day_and_hour_when_separator_is_dropped() {
        // OCR sometimes drops the separator entirely (observed on a real
        // Chinese screenshot): "Day 1529, 0851" read as bare "15290851".
        assert_eq!(extract_day_and_hour("15290851"), "1529, 08:51");
        assert_eq!(extract_day_and_hour("7020304"), "702, 03:04");
    }

    #[test]
    fn day_is_unbounded_only_time_is_fixed_width() {
        // Whatever the digit count, the last 4 are HH:MM and the rest is the day.
        assert_eq!(extract_day_and_hour("123456789"), "12345, 67:89");
    }

    #[test]
    fn rejects_timestamp_noise() {
        assert_eq!(extract_day_and_hour(""), "");
        assert_eq!(extract_day_and_hour("0851"), ""); // 4 digits: can't tell day from time
    }

    #[test]
    fn autocontrast_stretches_to_full_range() {
        // A low-contrast band [100, 140] should stretch to span [0, 255].
        let mut gray: Vec<u8> = (0..1000)
            .map(|i| 100 + (i % 41) as u8) // values in [100, 140]
            .collect();
        autocontrast(&mut gray, 0);
        assert_eq!(*gray.iter().min().unwrap(), 0);
        assert_eq!(*gray.iter().max().unwrap(), 255);
    }

    #[test]
    fn autocontrast_handles_flat_input() {
        // A uniform image has hi == lo; it must be left untouched, not divided by zero.
        let mut gray = vec![128u8; 256];
        autocontrast(&mut gray, 2);
        assert!(gray.iter().all(|&v| v == 128));
    }

    #[test]
    fn autocontrast_ignores_empty() {
        let mut gray: Vec<u8> = Vec::new();
        autocontrast(&mut gray, 2); // must not panic
        assert!(gray.is_empty());
    }

    #[test]
    fn trim_columns_crops_blank_margins_to_text_extent() {
        // 10-wide, 4-tall strip: a high-contrast column at x=3 and x=4 (text),
        // everything else flat (blank). With margin 1 the kept span is [2, 6).
        let width = 10;
        let height = 4;
        let mut gray = vec![0u8; width * height];
        // Text columns carry ink on some rows and background on others, giving
        // them a large vertical range; a fully-uniform column would not count.
        for &x in &[3usize, 4] {
            gray[width + x] = 255; // row 1
            gray[2 * width + x] = 255; // row 2
        }
        let (out, new_w) = trim_columns_to_content(&gray, width, height, 1);
        assert_eq!(new_w, 4); // cols 2,3,4,5
        assert_eq!(out.len(), new_w * height);
    }

    #[test]
    fn trim_columns_leaves_blank_strip_unchanged() {
        // No column clears the activity threshold: return the input untouched
        // rather than cropping to nothing.
        let gray = vec![40u8; 8 * 3];
        let (out, new_w) = trim_columns_to_content(&gray, 8, 3, 2);
        assert_eq!(new_w, 8);
        assert_eq!(out, gray);
    }

    #[test]
    fn pad_rows_expands_tight_crop_to_target_fraction() {
        // 4-wide, 10-tall strip with text filling rows 0..=7 (band 8 = 80% of
        // height). Targeting 50% must pad to height 16 (8 / 0.5) and keep the
        // original rows intact, centred.
        let width = 4;
        let height = 10;
        let gray = {
            let mut g = vec![0u8; width * height];
            for y in 0..8 {
                g[y * width] = 255;
            }
            g
        };
        let (out, new_h) = pad_rows_to_text_fraction(&gray, width, height, 0.5);
        assert_eq!(new_h, 16);
        assert_eq!(out.len(), width * new_h);
        // 6 padding rows split 3 top / 3 bottom: original rows land at 3..=12.
        assert!(
            out[..width * 3].iter().all(|&v| v == 0),
            "top padding is dark"
        );
        assert_eq!(
            out[3 * width],
            255,
            "first text row preserved after padding"
        );
    }

    #[test]
    fn pad_rows_is_noop_when_band_within_target() {
        // Text band already only 40% of height (rows 3..=6 of 10): no padding.
        let width = 4;
        let height = 10;
        let mut gray = vec![0u8; width * height];
        for y in 3..=6 {
            gray[y * width] = 255;
        }
        let (out, new_h) = pad_rows_to_text_fraction(&gray, width, height, 0.62);
        assert_eq!(new_h, height);
        assert_eq!(out, gray);
    }

    #[test]
    fn pad_rows_leaves_blank_strip_unchanged() {
        // No row clears the text threshold: return the input untouched.
        let gray = vec![20u8; 4 * 6];
        let (out, new_h) = pad_rows_to_text_fraction(&gray, 4, 6, 0.62);
        assert_eq!(new_h, 6);
        assert_eq!(out, gray);
    }

    /// Build a grayscale buffer where the given inclusive row ranges are "text"
    /// (a single bright pixel per row) and everything else is background.
    fn buffer_with_text_rows(width: usize, height: usize, rows: &[(usize, usize)]) -> Vec<u8> {
        let mut buf = vec![0u8; width * height];
        for &(start, end) in rows {
            for y in start..=end {
                buf[y * width] = 255; // one bright pixel marks the row as text
            }
        }
        buf
    }

    #[test]
    fn detect_text_bands_merges_single_block() {
        // One continuous block of text -> exactly one band.
        let buf = buffer_with_text_rows(10, 100, &[(20, 80)]);
        let bands = detect_text_bands(&buf, 10, 100);
        assert_eq!(bands.len(), 1);
    }

    #[test]
    fn detect_text_bands_splits_on_tall_gap() {
        // Two blocks separated by a tall blank gap -> two bands (wrapped name).
        let buf = buffer_with_text_rows(10, 120, &[(0, 30), (70, 100)]);
        let bands = detect_text_bands(&buf, 10, 120);
        assert_eq!(bands.len(), 2);
    }

    #[test]
    fn detect_text_bands_ignores_small_intra_glyph_gap() {
        // A short blank run inside a glyph must not split the band.
        let buf = buffer_with_text_rows(10, 100, &[(20, 50), (53, 80)]); // 2-row gap
        let bands = detect_text_bands(&buf, 10, 100);
        assert_eq!(bands.len(), 1);
    }

    #[test]
    fn split_text_lines_returns_whole_buffer_for_single_row() {
        let buf = buffer_with_text_rows(10, 100, &[(20, 80)]);
        let lines = split_text_lines(&buf, 10, 100);
        assert_eq!(lines.len(), 1);
        assert_eq!((lines[0].1, lines[0].2), (10, 100)); // unchanged dimensions
    }

    #[test]
    fn join_lines_horizontally_places_rows_side_by_side() {
        // Two 4x4 white tiles -> width grows, height is single row + padding.
        let a = (vec![255u8; 16], 4, 4);
        let b = (vec![255u8; 16], 4, 4);
        let (canvas, w, h) = join_lines_horizontally(&[a, b]);
        assert_eq!(w, 4 + 4 + 16 + 2 * 30); // widths + GAP + 2*PAD
        assert_eq!(h, 4 + 2 * 30); // tallest row + 2*PAD
        assert_eq!(canvas.len(), w * h);
        // The quiet-zone border stays background (dark).
        assert_eq!(canvas[0], 0);
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

//! Main scanning pipeline orchestration.
//!
//! Coordinates the detection, OCR, and template matching stages
//! to produce a complete stockpile scan result.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Maximum total boxes to process (prevents DoS via excessive memory allocation).
/// A typical Foxhole stockpile has 6 columns × ~10 rows = 60 items max per view.
const MAX_TOTAL_BOXES: usize = 200;

use rayon::prelude::*;

use super::debug_ocr;
use super::metadata_parse::{
    extract_day_and_hour, is_public_default_name, match_shard_name, ClientLanguage,
    PUBLIC_CANONICAL_NAME,
};
use super::region_preprocess::{
    extract_region, join_lines_horizontally, preprocess_for_recognizer, split_text_lines,
    PreprocessParams,
};
use crate::config::ScanConfig;
use crate::detector::{BlackBoxDetector, DetectedRegions, GreyMaskDetector};
use crate::enums::ItemFaction;
use crate::enums::StockpileType;
use crate::error::{FsOcrError, Result};
use crate::image_utils;
use crate::models::{ItemCandidate, Stockpile, StockpileItem, Timing};
use crate::ocr::{digit_matcher, ChineseNameReader, TextExtractor};
use crate::template::database::TemplateDatabase;
use crate::template::matching::{MatchFilter, MatchResult, TemplateMatcher};
use crate::template::phash::compute_phash;

/// Decode-time character masks for the ocrs backend (see `OcrConfig::allowed_chars`).
/// Restricting the recognizer to a field's plausible character set keeps
/// closed-vocabulary reads on-script (e.g. a Latin shard never decodes to
/// Cyrillic) and stops marker words from being hallucinated as stray digits.
///
/// Shard names are a fixed Latin set (ABLE / CHARLIE / DevBranch / LIVE).
const SHARD_MASK: &str = "ABCDEHILRVacehnrv";

/// Timestamp masks are per script, matching the 3-way `ClientLanguage`. Each
/// allows digits, spaces and the separators (`, . : -`) plus that script's
/// letters, so the localized `Day`/`Hours` markers decode as letters (which the
/// parser discards) instead of corrupting the digit run. Latin uses the full
/// alphabet so any Latin client (EN/DE/FR/PT/…) reads correctly.
pub(crate) const TIME_MASK_LATIN: &str =
    "0123456789 ,.:-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
pub(crate) const TIME_MASK_CYRILLIC: &str =
    "0123456789 ,.:-АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя";
pub(crate) const TIME_MASK_CHINESE: &str = "0123456789,，日时分";

/// Per-script masks for the stockpile-type region. The localized type strings
/// are a closed vocabulary that never mixes scripts within one label, so each
/// mask carries only its script's letters (plus space and the hyphen in
/// "BMS - Longhook"). Decoding the region under one mask at a time keeps the
/// read on-script — without this an English label like "Aircraft Depot" decodes
/// with stray Cyrillic glyphs ("X'rcraft Dcьзr") and matches nothing. The Latin
/// mask carries the accents the EN/DE/FR/PT names use (Dépôt, Torreão, …).
const TYPE_MASK_LATIN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz -àâãçèéíóôú";
const TYPE_MASK_CYRILLIC: &str =
    " АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя";
const TYPE_MASK_CHINESE: &str = "营地要塞安全屋遗迹基堡边境城镇下仓库海港";

/// Type-region decode cascade, tried in order. Latin first (the common client
/// and the script most prone to cross-script intrusion), then Cyrillic, then
/// Chinese; the first mask whose decode matches a known type wins, and its
/// script tells us the client language for routing the shard/timestamp block.
const TYPE_MASKS: &[(&str, ClientLanguage)] = &[
    (TYPE_MASK_LATIN, ClientLanguage::English),
    (TYPE_MASK_CYRILLIC, ClientLanguage::Russian),
    (TYPE_MASK_CHINESE, ClientLanguage::Chinese),
];

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
    /// ocrs extractor for Latin/Cyrillic custom stockpile names.
    text_extractor: Option<TextExtractor>,
    /// Lazily-built ocrs extractors keyed by their `allowed_chars` mask. A scan
    /// touches at most two masks (the fixed shard mask plus one script's
    /// timestamp mask), so this caches them across scans without reloading the
    /// recognition model more than once per distinct mask.
    masked_extractors: Mutex<HashMap<String, TextExtractor>>,
    /// Optional system-`tesseract` reader for Chinese custom names, probed once
    /// on first Chinese name encountered. Absent install -> names left unread.
    chinese_name_reader: OnceLock<ChineseNameReader>,
}

impl ScanPipeline {
    /// Create a new scan pipeline.
    pub fn new<P: AsRef<Path>>(database_path: P, data_path: P, config: ScanConfig) -> Self {
        Self {
            database_path: database_path.as_ref().to_string_lossy().to_string(),
            data_path: data_path.as_ref().to_string_lossy().to_string(),
            config,
            database: None,
            text_extractor: None,
            masked_extractors: Mutex::new(HashMap::new()),
            chinese_name_reader: OnceLock::new(),
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

        // Initialize the ocrs extractor used for Latin/Cyrillic custom names.
        // (The model name is cosmetic for ocrs — it always loads the embedded
        // recognition model — so a single "eng" extractor suffices.)
        if self.text_extractor.is_none() {
            match TextExtractor::new_for_text_default("eng") {
                Ok(extractor) => self.text_extractor = Some(extractor),
                Err(e) => {
                    eprintln!("Warning: Failed to initialize text OCR: {}", e);
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
    /// no argument is needed — there are only three scripts.
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
            let (processed, proc_w, proc_h) = preprocess_for_recognizer(
                &type_img,
                w as usize,
                h as usize,
                1,
                &PreprocessParams::light_text(scale_factor, 2.0),
            );

            if debug_ocr::enabled() {
                debug_ocr::save_gray("type", &processed, proc_w, proc_h);
            }

            let (stockpile_type, lang) =
                self.recognize_type_text(&processed, proc_w as i32, proc_h as i32);
            client_language = lang;
            stockpile.stockpile_type = stockpile_type;
        }

        // Extract shard and ingame timestamp. The region holds 2 lines:
        // timestamp on top, shard name on bottom. The timestamp is read with the
        // client language's per-script mask (detected from the type above).
        if let Some((x, y, w, h)) = regions.shard_region {
            self.read_shard_region(
                image,
                width,
                height,
                (x, y, w, h),
                client_language,
                stockpile,
            );
        }

        // Extract stockpile name (only for types that support custom names).
        // Latin/Cyrillic names read via ocrs; Chinese names via the optional
        // tesseract CLI (see the name branch below).
        if stockpile.stockpile_type.has_custom_name() {
            let mut name_region_present = false;
            let mut is_public = false;
            if let Some((x, y, w, h)) = regions.name_region {
                name_region_present = true;
                let name_img = extract_region(
                    image,
                    width as usize,
                    height as usize,
                    x.max(0) as usize,
                    y.max(0) as usize,
                    w as usize,
                    h as usize,
                );

                // Preprocess with extra upscale for better name detection.
                let (processed, proc_w, proc_h) = preprocess_for_recognizer(
                    &name_img,
                    w as usize,
                    h as usize,
                    1,
                    &PreprocessParams::name(scale_factor, 4.0),
                );

                if debug_ocr::enabled() {
                    debug_ocr::save_gray("name", &processed, proc_w, proc_h);
                }

                // Primary: is this the game's localized public default? Matched as
                // a template, so it is recognized in every language — including
                // zh/ru, whose custom names the recognizer can't read.
                if let Some(lang) =
                    crate::template::public_match::match_public_label(&processed, proc_w, proc_h)
                {
                    if debug_ocr::enabled() {
                        eprintln!("[FS_DEBUG_OCR] name matched public default ({lang:?})");
                    }
                    stockpile.name = Some(PUBLIC_CANONICAL_NAME.to_string());
                    is_public = true;
                } else {
                    // A custom (reserved) name. Read with the recognizer that
                    // fits the client's script; a failed/absent read leaves the
                    // name unset (still flagged reserved below).
                    let name = if client_language == ClientLanguage::Chinese {
                        // Chinese custom names fall outside the ocrs alphabet, so
                        // read them with the system `tesseract` CLI when it's
                        // installed; otherwise the name is left unread.
                        self.chinese_name_reader
                            .get_or_init(ChineseNameReader::new)
                            .read(&processed, proc_w as u32, proc_h as u32)
                    } else {
                        self.read_custom_name_ocrs(&processed, proc_w, proc_h)
                    };
                    if let Some(name) = name {
                        let name = name.trim();
                        if !name.is_empty() {
                            stockpile.name = Some(name.to_string());
                        }
                    }
                }
            }

            // Fallback for Latin clients: an OCR misread of the public default
            // that the template missed (l/I, dropped accent) still normalizes to
            // the canonical label rather than being treated as a custom name.
            if !is_public
                && stockpile
                    .name
                    .as_deref()
                    .is_some_and(is_public_default_name)
            {
                stockpile.name = Some(PUBLIC_CANONICAL_NAME.to_string());
                is_public = true;
            }

            // A present name region that isn't the public default is a custom
            // (reserved) name — even one we couldn't read (e.g. a Chinese name
            // with no tesseract installed).
            stockpile.is_reserved = name_region_present && !is_public;
        }

        Ok(())
    }

    /// Read a Latin/Cyrillic custom name from a preprocessed crop with ocrs.
    ///
    /// The game wraps long names across two rows; genuine row wrapping (a tall
    /// blank gap) is detected and the rows are laid side by side into one logical
    /// line before recognition, so the recognizer gets line context and the
    /// original name is reconstructed. Returns `None` when no extractor is loaded.
    fn read_custom_name_ocrs(
        &self,
        processed: &[u8],
        proc_w: usize,
        proc_h: usize,
    ) -> Option<String> {
        let extractor = self.text_extractor.as_ref()?;

        let lines = split_text_lines(processed, proc_w, proc_h);
        if debug_ocr::enabled() && lines.len() > 1 {
            for (i, (buf, lw, lh)) in lines.iter().enumerate() {
                debug_ocr::save_gray(&format!("name_line{i}"), buf, *lw, *lh);
            }
        }

        let (ocr_img, ocr_w, ocr_h) = if lines.len() > 1 {
            join_lines_horizontally(&lines)
        } else {
            (processed.to_vec(), proc_w, proc_h)
        };

        if debug_ocr::enabled() && lines.len() > 1 {
            debug_ocr::save_gray("name_merged", &ocr_img, ocr_w, ocr_h);
        }

        extractor
            .extract_text(&ocr_img, ocr_w as i32, ocr_h as i32, 1)
            .ok()
    }

    /// Recognize the stockpile type and infer the client language from a
    /// preprocessed type-region crop.
    ///
    /// ocrs (default): decode the region under each script mask in [`TYPE_MASKS`]
    /// in turn (Latin, then Cyrillic, then Chinese) and return the first that
    /// matches a known type, along with that script's client language. Masking
    /// keeps each pass on-script so a clean Latin label isn't lost to a stray
    /// Cyrillic homoglyph. If no mask yields a known type, the type is
    /// `Undefined` and the language defaults to English (Latin routing).
    fn recognize_type_text(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
    ) -> (StockpileType, ClientLanguage) {
        // Primary: template-match the rendered type labels. Language-agnostic and
        // independent of the recognizer charset, so it covers every locale.
        if let Some(m) = Self::match_type_template(image, width, height) {
            return m;
        }

        // Fallback: per-script OCR decode cascade.
        for &(mask, lang) in TYPE_MASKS {
            let Ok(text) = self.extract_with_mask(mask, image, width, height) else {
                continue;
            };
            if debug_ocr::enabled() {
                eprintln!(
                    "[FS_DEBUG_OCR] type region ({:?} mask) raw text: {:?}",
                    lang, text
                );
            }
            let stockpile_type = StockpileType::from_string(&text);
            if stockpile_type != StockpileType::Undefined {
                return (stockpile_type, lang);
            }
        }
        (StockpileType::Undefined, ClientLanguage::English)
    }

    /// Template-match the preprocessed type crop against the embedded label
    /// renders, returning the type and the script routing for its language.
    /// `None` when nothing clears the match floor (the caller falls back to OCR).
    fn match_type_template(
        image: &[u8],
        width: i32,
        height: i32,
    ) -> Option<(StockpileType, ClientLanguage)> {
        let m = crate::template::type_match::match_type_label(
            image,
            width.max(0) as usize,
            height.max(0) as usize,
        )?;
        if debug_ocr::enabled() {
            eprintln!(
                "[FS_DEBUG_OCR] type template match: {:?} ({:?}) score={:.3}",
                m.stype, m.lang, m.score
            );
        }
        Some((m.stype, ClientLanguage::from_game(m.lang)))
    }

    /// Run the ocrs recognizer over `image` with a decode mask, building and
    /// caching one masked extractor per distinct `allowed_chars` string. The
    /// cache lock is held across the recognition call, which is fine: ocrs runs
    /// single-threaded (RTEN_NUM_THREADS=1) and scans are serial.
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

    /// Read the timestamp and shard name from the shard region.
    ///
    /// ocrs recognizes a single rect with no line detection, so a 2-line crop
    /// collapses into garbage. We split the region into its top (timestamp) and
    /// bottom (shard) halves and read each as a single line with a decode mask:
    /// the timestamp uses the client's script mask, the shard the fixed Latin
    /// shard mask. CJK/Cyrillic *timestamps* are out of scope for this Latin
    /// recognizer; only the digits within them tend to survive.
    fn read_shard_region(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        region: (i32, i32, i32, i32),
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
        let (processed, proc_w, proc_h) = preprocess_for_recognizer(
            &timestamp_img,
            w as usize,
            half_h as usize,
            1,
            &PreprocessParams::strip(),
        );

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
        let (processed, proc_w, proc_h) = preprocess_for_recognizer(
            &shard_img,
            w as usize,
            half_h as usize,
            1,
            &PreprocessParams::strip(),
        );

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

                        build_matched_item(&match_result, quantity)
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
                        build_matched_item(&match_result, quantity)
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

/// Build a matched StockpileItem from a successful match result.
fn build_matched_item(match_result: &MatchResult, quantity: i32) -> StockpileItem {
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

#[cfg(test)]
mod tests {
    use super::*;

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

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
use crate::config::ScanConfig;
use crate::detector::{BlackBoxDetector, DetectedRegions, GreyMaskDetector};
use crate::enums::GameLanguage;
use crate::enums::ItemFaction;
use crate::enums::StockpileType;
use crate::error::{FsOcrError, Result};
use crate::image_utils;
use crate::models::{ItemCandidate, Stockpile, StockpileItem, Timing};
use crate::ocr::{digit_matcher, preprocess, ChineseNameReader, TextExtractor};
use crate::template::database::TemplateDatabase;
use crate::template::matching::{MatchFilter, TemplateMatcher};
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
const TIME_MASK_LATIN: &str = "0123456789 ,.:-ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const TIME_MASK_CYRILLIC: &str =
    "0123456789 ,.:-АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя";
const TIME_MASK_CHINESE: &str = "0123456789,，日时分";

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

        let (processed, proc_w, proc_h) = preprocess_for_recognizer(
            &region_img,
            w as usize,
            h as usize,
            1,
            &PreprocessParams::light_text(scale_factor, 2.0),
        );

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

/// Upscale strategy for the recognizer preprocessing.
enum Upscale {
    /// Continuous bilinear scale by `base / scale_factor` (resolution-driven).
    /// Used for the type and name fields.
    Continuous { base: f64, scale_factor: f64 },
    /// Integer factor toward a target per-line height (crop-driven). Used for
    /// the shard/timestamp strips, which stack `lines` text rows.
    LineHeight,
}

/// Vertical text-to-frame ratio every single-line recognizer crop is padded to.
/// The model is trained on this exact framing, so the training generator
/// (`training/generate_dataset.py`) MUST render at the same value — keep the two
/// in sync or the recognizer sees text at a scale it never learned.
const TEXT_FRAME_RATIO: f64 = 0.60;

/// Horizontal quiet zone added on each side of the text, as a fraction of the
/// text-band height. Small but non-zero: a tight crop removes the variable blank
/// margin (which the recognizer otherwise reads as a phantom edge glyph — e.g.
/// the doubled leading `O` in `OORCA`), and this constant adds back just enough
/// uniform breathing room. The generator renders the same quiet zone.
const QUIET_ZONE_RATIO: f64 = 0.15;

/// Floor for the quiet zone (px), so tiny crops still keep a 2px margin.
const MIN_QUIET_ZONE: usize = 2;

/// A row/column carries ink when its luma range (max − min) clears this; ~a
/// quarter of the full 0..=255 range that autocontrast stretches to. Used by
/// both the column trim and the canonical single-line framing, and polarity-
/// agnostic by construction (it keys off contrast, not absolute brightness).
const ACTIVITY_THRESHOLD: u8 = 64;

/// The only per-field difference left in the recognizer input: how the canonical
/// frame is scaled up toward a legible size. Everything else — luma, autocontrast,
/// polarity normalization, the tight-crop-then-pad framing — is identical across
/// type, name, shard, and timestamp so a single canonical preprocessing serves
/// every field (and, modulo a final polarity flip, either OCR backend).
struct PreprocessParams {
    upscale: Upscale,
}

impl PreprocessParams {
    /// Type/name banner line: scale continuously by `upscale_base / scale_factor`
    /// (2.0 for the type banner, 4.0 for the smaller name line).
    fn light_text(scale_factor: f64, upscale_base: f64) -> Self {
        Self {
            upscale: Upscale::Continuous {
                base: upscale_base,
                scale_factor,
            },
        }
    }

    /// Stockpile name line. Same canonical framing as every other field; named
    /// for call-site clarity.
    fn name(scale_factor: f64, upscale_base: f64) -> Self {
        Self::light_text(scale_factor, upscale_base)
    }

    /// Shard/timestamp strip: upscale toward a legible per-line height.
    fn strip() -> Self {
        Self {
            upscale: Upscale::LineHeight,
        }
    }
}

/// Shared preprocessing for every field read by the ocrs recognizer (type,
/// name, shard, timestamp). The step order is fixed; `params` selects the
/// optional polarity/padding steps and the upscale strategy so each field keeps
/// its established behavior behind a single code path.
fn preprocess_for_recognizer(
    image: &[u8],
    width: usize,
    height: usize,
    lines: usize,
    params: &PreprocessParams,
) -> (Vec<u8>, usize, usize) {
    // Standard luma conversion: 0.299*R + 0.587*G + 0.114*B.
    let mut processed = Vec::with_capacity(width * height);
    for chunk in image.chunks_exact(3) {
        let luma =
            ((77u16 * chunk[0] as u16 + 150u16 * chunk[1] as u16 + 29u16 * chunk[2] as u16 + 128)
                >> 8) as u8;
        processed.push(luma);
    }

    // Stretch contrast so text becomes legible regardless of base brightness;
    // low-contrast grey-on-grey names and bright info bars both normalize to the
    // full dynamic range.
    autocontrast(&mut processed, 2);

    // Normalize polarity to light-on-dark. The recognizer is trained light-on-
    // dark; an in-game theme can render any field dark-on-light, which decodes to
    // junk if fed uninverted. After autocontrast the background dominates one
    // extreme, so a bright mean means dark-text-on-light: flip it.
    let mean: u32 =
        processed.iter().map(|&v| v as u32).sum::<u32>() / processed.len().max(1) as u32;
    if mean > 127 {
        for v in processed.iter_mut() {
            *v = 255 - *v;
        }
    }

    // Canonical framing. A single line is tight-cropped to its ink bbox on both
    // axes and re-padded to TEXT_FRAME_RATIO with a quiet zone — the exact frame
    // the model is trained on, and free of the variable blank margin the
    // detection box carries (which the recognizer otherwise reads as a phantom
    // edge glyph). A multi-line strip keeps its stacked rows; only the horizontal
    // margin is tightened, since per-row vertical banding isn't meaningful across
    // stacked lines.
    let (processed, width, height) = if lines == 1 {
        fit_single_line_frame(&processed, width, height)
    } else {
        let band_h = (height / lines.max(1)).max(1);
        let qz = (((band_h as f64) * QUIET_ZONE_RATIO).round() as usize).max(MIN_QUIET_ZONE);
        let (cropped, new_w) = trim_columns_to_content(&processed, width, height, qz);
        (cropped, new_w, height)
    };

    apply_upscale(processed, width, height, lines, &params.upscale)
}

/// Scale the trimmed crop up per the chosen `Upscale` strategy.
fn apply_upscale(
    buf: Vec<u8>,
    width: usize,
    height: usize,
    lines: usize,
    upscale: &Upscale,
) -> (Vec<u8>, usize, usize) {
    match *upscale {
        Upscale::Continuous { base, scale_factor } => {
            let factor = base / scale_factor;
            let new_w = ((width as f64) * factor) as usize;
            let new_h = ((height as f64) * factor) as usize;
            let scaled = preprocess::upscale_bilinear(&buf, width, height, new_w, new_h);
            (scaled, new_w, new_h)
        }
        Upscale::LineHeight => {
            // Target a legible per-line height. At low resolutions a single line
            // can be ~13px tall, below what the model reads reliably; the factor
            // targets a per-line height rather than blindly multiplying, since
            // over-upscaling blurs and hurts OCR.
            const TARGET_LINE_HEIGHT: usize = 26;
            let line_height = (height / lines.max(1)).max(1);
            let factor = ((TARGET_LINE_HEIGHT + line_height / 2) / line_height).max(1);
            if factor > 1 {
                let new_w = width * factor;
                let new_h = height * factor;
                let scaled = preprocess::upscale_bilinear(&buf, width, height, new_w, new_h);
                (scaled, new_w, new_h)
            } else {
                (buf, width, height)
            }
        }
    }
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

/// Crop a single-line grayscale crop tight to its ink bounding box on both axes,
/// then re-pad to the canonical frame: a horizontal quiet zone of
/// [`QUIET_ZONE_RATIO`] × band-height on each side, and top/bottom padding so the
/// text band fills [`TEXT_FRAME_RATIO`] of the height. Background is dark (0)
/// after polarity normalization, so padding is dark.
///
/// This is the heart of the shared preprocessing: it strips the variable blank
/// margin the detection box carries (which the recognizer otherwise reads as a
/// phantom leading/trailing glyph — the doubled `O` in `OORCA`) and presents
/// every field at the one constant scale and framing the model is trained on.
///
/// Ink is detected by per-row / per-column luma *range* (max − min ≥
/// [`ACTIVITY_THRESHOLD`]), which is polarity-agnostic. A genuinely blank crop
/// (no row or column clears the threshold) is returned unchanged rather than
/// cropped to nothing.
///
/// The type region is cropped tight to its text slab upstream (in the detector),
/// so its crop is already text-only. The name region's layout varies
/// (pinned/unpinned/old-format) and can still carry a stray bright band, so the
/// vertical extent is taken from [`dominant_text_band`] — the brightest
/// contiguous stroke run — rather than a plain first..last span that a detached
/// band would inflate.
///
/// Fraction of the crop's peak brightness a pixel must reach to count as a text
/// stroke. Strokes are near-white after autocontrast + polarity normalization; a
/// dim noise gradient stays mid-grey and never clears this.
const INK_PIXEL_RATIO: f64 = 0.55;

/// Find the vertical [first, last] row span of the actual text line.
///
/// Assumes light-on-dark (the framing runs after polarity normalization). A row
/// belongs to the text when it carries bright *stroke* pixels (value ≥
/// [`INK_PIXEL_RATIO`] × the crop's peak); this is true even for ascender/cap
/// rows whose strokes are thin (low row mean) but bright, so they are kept — and
/// false for a dim noise gradient bleeding into the crop, which has no near-white
/// pixels and so forms a separate, non-inked gap. Among the contiguous inked runs
/// the one carrying the most stroke pixels wins, dropping a detached noise band;
/// the chosen run is then grown across any row that still carries a stroke pixel,
/// so thin cap/ascender tips and descender tails aren't clipped.
/// Returns `None` when no row carries ink (a blank crop).
fn dominant_text_band(gray: &[u8], width: usize, height: usize) -> Option<(usize, usize)> {
    if width == 0 || height == 0 {
        return None;
    }

    let peak = gray.iter().copied().max().unwrap_or(0);
    if peak == 0 {
        return None;
    }
    let ink_level = (peak as f64 * INK_PIXEL_RATIO).round() as u8;
    // A handful of bright pixels marks a real stroke row while ignoring isolated
    // sensor speckle; scales gently with width so it works at every resolution.
    let min_ink = (width / 128).max(2);

    let row_ink = |y: usize| -> usize {
        gray[y * width..y * width + width]
            .iter()
            .filter(|&&v| v >= ink_level)
            .count()
    };

    let mut best: Option<(usize, usize)> = None;
    let mut best_energy = 0usize;
    let mut run_start: Option<usize> = None;
    let mut run_energy = 0usize;
    for y in 0..=height {
        let ink = if y < height { row_ink(y) } else { 0 };
        if ink >= min_ink {
            run_start.get_or_insert(y);
            run_energy += ink;
        } else if let Some(start) = run_start.take() {
            if run_energy > best_energy {
                best_energy = run_energy;
                best = Some((start, y - 1));
            }
            run_energy = 0;
        }
    }

    // Grow the chosen run across any row that still carries a stroke pixel, so a
    // cap/ascender tip or descender tail (one or two bright pixels, below
    // `min_ink`) isn't clipped. The noise band has no near-white pixels, so this
    // stops at the blank gap before it rather than swallowing it.
    let (mut y0, mut y1) = best?;
    while y0 > 0 && row_ink(y0 - 1) >= 1 {
        y0 -= 1;
    }
    while y1 + 1 < height && row_ink(y1 + 1) >= 1 {
        y1 += 1;
    }
    Some((y0, y1))
}

fn fit_single_line_frame(gray: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    if width == 0 || height == 0 {
        return (gray.to_vec(), width, height);
    }

    // Columns carrying ink (large vertical luma range).
    let mut x_first: Option<usize> = None;
    let mut x_last = 0usize;
    for x in 0..width {
        let (mut min, mut max) = (255u8, 0u8);
        for y in 0..height {
            let v = gray[y * width + x];
            min = min.min(v);
            max = max.max(v);
        }
        if max - min >= ACTIVITY_THRESHOLD {
            x_first.get_or_insert(x);
            x_last = x;
        }
    }

    // Vertical extent: the brightest contiguous stroke run, so a stray bright
    // band in a variable-layout name crop doesn't inflate the frame.
    let Some((y0, y_last)) = dominant_text_band(gray, width, height) else {
        return (gray.to_vec(), width, height);
    };
    let Some(x0) = x_first else {
        return (gray.to_vec(), width, height);
    };
    let band_w = x_last - x0 + 1;
    let band_h = y_last - y0 + 1;

    let quiet = (((band_h as f64) * QUIET_ZONE_RATIO).round() as usize).max(MIN_QUIET_ZONE);
    let desired_h = (((band_h as f64) / TEXT_FRAME_RATIO).round() as usize).max(band_h);
    let top = (desired_h - band_h) / 2;
    let new_w = band_w + 2 * quiet;

    let mut out = vec![0u8; new_w * desired_h];
    for ry in 0..band_h {
        let src = (y0 + ry) * width + x0;
        let dst = (top + ry) * new_w + quiet;
        out[dst..dst + band_w].copy_from_slice(&gray[src..src + band_w]);
    }
    (out, new_w, desired_h)
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

/// Client UI language, inferred from the stockpile type (via the matching
/// [`TYPE_MASKS`] entry or type-template match).
///
/// Routes the timestamp decode mask to the right script and decides whether a
/// custom name is read via ocrs (Latin/Cyrillic) or the tesseract CLI (Chinese).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientLanguage {
    English,
    Chinese,
    Russian,
}

impl ClientLanguage {
    /// Collapse a fine-grained [`GameLanguage`] (from a type-template match) to
    /// the script routing used for the shard/timestamp block: the Latin locales
    /// all read as English, Russian as Cyrillic, Chinese as Han.
    fn from_game(lang: GameLanguage) -> Self {
        match lang {
            GameLanguage::Russian => ClientLanguage::Russian,
            GameLanguage::Chinese => ClientLanguage::Chinese,
            _ => ClientLanguage::English,
        }
    }

    /// The timestamp decode mask for this client's script.
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

/// Localized game labels for the default (non-custom) *public* stockpile name,
/// lowercased for comparison. A name region that reads as one of these is the
/// game's auto label for a public stockpile, not a user-chosen reserve name.
///
/// Latin scripts only (English/French share `public`): the recognizer's charset
/// no longer carries Chinese or Cyrillic, so it can never emit `公共` /
/// `Публичный` — those would be permanently-dead entries that falsely imply
/// support we've dropped.
const PUBLIC_DEFAULT_NAMES: &[&str] = &["public", "público", "öffentlich"];

/// Canonical label stored when a name matches a public default, regardless of
/// the client's language or the OCR noise that reached us.
const PUBLIC_CANONICAL_NAME: &str = "Public";

/// Whether an OCR'd name is the localized public default rather than a custom
/// reserve name.
///
/// Matched fuzzily so the geometric game font's `l`/`I` collision (e.g. `Public`
/// read as `PubIic`) and a dropped accent (`Público` → `Publico`) still resolve.
/// The `0.80` floor accepts ~one edit on the shortest entry (`public`, 6 chars:
/// one substitution scores `1 - 1/6 ≈ 0.83`) while staying tight enough that an
/// arbitrary custom name never collapses into the default.
fn is_public_default_name(name: &str) -> bool {
    const MIN_SIMILARITY: f64 = 0.80;

    let candidate = name.trim().to_lowercase();
    if candidate.is_empty() {
        return false;
    }
    PUBLIC_DEFAULT_NAMES
        .iter()
        .any(|&default| crate::text_utils::similarity(&candidate, default) >= MIN_SIMILARITY)
}

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
/// Every locale separates the day from the time with a comma — ASCII `,` for
/// Latin/Cyrillic clients, fullwidth `，` (U+FF0C) for Chinese — and it always
/// falls AFTER any thousands separator inside the day. So we split on the LAST
/// comma: digits to its left are the day, and the FIRST four digits to its right
/// are HHMM. Taking the first four (not the trailing four of the whole string)
/// means digits leaked from a misread trailing marker word — e.g. "Hours" read
/// as "Hour5" — can't shift the time window.
///
/// If OCR drops the separator entirely we fall back to "the last 4 digits are
/// HHMM, the rest is the day". Either way the result is rejected unless it parses
/// to a real clock time (HH 00-23, MM 00-59), so a misread digit yields no
/// timestamp rather than a confidently-wrong one.
fn extract_day_and_hour(text: &str) -> String {
    let (day, hhmm) = if let Some(sep) = text.rfind([',', '，']) {
        let day: String = text[..sep].chars().filter(|c| c.is_ascii_digit()).collect();
        let hhmm: String = text[sep..]
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(4)
            .collect();
        (day, hhmm)
    } else {
        // No separator: the last 4 digits are HHMM, the rest the day. Needs at
        // least the 4 time digits plus 1 day digit.
        let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 5 {
            return String::new();
        }
        let split = digits.len() - 4;
        (digits[..split].to_string(), digits[split..].to_string())
    };

    if day.is_empty() || hhmm.len() != 4 {
        return String::new();
    }

    // The in-game clock is HH 00-23, MM 00-59. A value outside that range means a
    // digit was misread, so reject the read instead of emitting a wrong time.
    let (hh, mm) = (&hhmm[..2], &hhmm[2..]);
    let in_range =
        hh.parse::<u32>().is_ok_and(|h| h < 24) && mm.parse::<u32>().is_ok_and(|m| m < 60);
    if !in_range {
        return String::new();
    }

    format!("{}, {}:{}", day, hh, mm)
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
    fn public_default_matches_localized_labels() {
        // English/French, Portuguese, German defaults all read as public.
        assert!(is_public_default_name("Public"));
        assert!(is_public_default_name("público"));
        assert!(is_public_default_name("Öffentlich"));
    }

    #[test]
    fn public_default_tolerates_ocr_noise() {
        // The geometric-font l/I collision and a dropped accent must still match.
        assert!(is_public_default_name("PubIic")); // l read as capital I
        assert!(is_public_default_name("Publico")); // ó read without the accent
        assert!(is_public_default_name("  Public  ")); // surrounding whitespace
    }

    #[test]
    fn public_default_rejects_custom_and_empty_names() {
        assert!(!is_public_default_name("ABC DEF GH"));
        assert!(!is_public_default_name("ORCA-THR-C"));
        assert!(!is_public_default_name("Publish")); // 2 edits from "public"
        assert!(!is_public_default_name(""));
        assert!(!is_public_default_name("   "));
    }

    #[test]
    fn public_default_excludes_dropped_scripts() {
        // Chinese/Russian support is dropped, so the recognizer can never emit
        // these and they are deliberately absent from the dictionary.
        assert!(!is_public_default_name("公共"));
        assert!(!is_public_default_name("Публичный"));
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
    fn extracts_day_and_hour_across_latin_locales() {
        // German/Portuguese use a period thousands separator, so the only comma
        // is the day/time split; French/English use a comma there. All read the
        // first 4 digits after the last comma as HHMM.
        assert_eq!(
            extract_day_and_hour("Tag 1.293, 1906 Stunden"),
            "1293, 19:06"
        );
        assert_eq!(
            extract_day_and_hour("Jour 1,293, 1906 Heures"),
            "1293, 19:06"
        );
        assert_eq!(extract_day_and_hour("Dia 1.293, 1906 Horas"), "1293, 19:06");
    }

    #[test]
    fn time_ignores_digits_leaked_from_a_misread_marker_word() {
        // Real misread: "Hours" recognized as "Hour5". Stripping all digits and
        // taking the trailing 4 used to yield "4181, 03:85"; splitting on the
        // comma and taking the FIRST 4 digits on the right reads it correctly.
        assert_eq!(extract_day_and_hour("Day 418, 1038 Hour5"), "418, 10:38");
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
        // Whatever the day's digit count, the last 4 (no separator) are HH:MM and
        // the rest is the day — as long as the time is a valid clock value.
        assert_eq!(
            extract_day_and_hour("Day 12345, 2030 Hours"),
            "12345, 20:30"
        );
    }

    #[test]
    fn rejects_timestamp_noise() {
        assert_eq!(extract_day_and_hour(""), "");
        assert_eq!(extract_day_and_hour("0851"), ""); // 4 digits: can't tell day from time
    }

    #[test]
    fn rejects_impossible_clock_values() {
        // A misread digit that makes the time impossible (HH>23 or MM>59) is
        // rejected rather than emitted as a wrong-but-plausible timestamp.
        assert_eq!(extract_day_and_hour("Day 418, 1085 Hours"), ""); // MM 85
        assert_eq!(extract_day_and_hour("Day 418, 2538 Hours"), ""); // HH 25
        assert_eq!(extract_day_and_hour("123456789"), ""); // no comma -> 67:89, invalid
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
    fn fit_frame_crops_to_ink_and_pads_to_ratio() {
        // 12-wide, 12-tall crop with a high-contrast 2x2 ink block at rows 5..=6,
        // cols 4..=5 (band 2x2). Expect: tight to the 2x2, quiet zone =
        // max(2, round(2*0.15)) = 2 px each side -> width 2 + 4 = 6; vertical pad
        // to round(2 / 0.60) = 3 rows tall, band centred (1 row top pad).
        let width = 12;
        let height = 12;
        let mut gray = vec![0u8; width * height];
        for y in 5..=6 {
            for x in 4..=5 {
                gray[y * width + x] = 255;
            }
        }
        let (out, new_w, new_h) = fit_single_line_frame(&gray, width, height);
        assert_eq!(
            (new_w, new_h),
            (6, 3),
            "tight crop + quiet zone + ratio pad"
        );
        assert_eq!(out.len(), new_w * new_h);
        // desired_h = round(2/0.60) = 3, top pad = (3-2)/2 = 0: the single pad row
        // lands at the bottom. Ink sits inside the 2px quiet zone (cols 2..=3).
        assert!(
            out[2 * new_w..].iter().all(|&v| v == 0),
            "bottom row is dark padding"
        );
        assert_eq!(
            out[2], 255,
            "ink starts inside the quiet zone on the first row"
        );
    }

    #[test]
    fn fit_frame_leaves_blank_crop_unchanged() {
        // No column/row clears the activity threshold: return the input untouched
        // rather than cropping to nothing.
        let gray = vec![20u8; 8 * 6];
        let (out, new_w, new_h) = fit_single_line_frame(&gray, 8, 6);
        assert_eq!((new_w, new_h), (8, 6));
        assert_eq!(out, gray);
    }

    #[test]
    fn presets_share_canonical_framing_and_differ_only_in_upscale() {
        // Every field flows through the one canonical preprocessing; the only
        // per-field knob left is the upscale strategy.
        match PreprocessParams::light_text(0.5, 4.0).upscale {
            Upscale::Continuous { base, scale_factor } => {
                assert_eq!((base, scale_factor), (4.0, 0.5));
            }
            Upscale::LineHeight => panic!("light_text must use a continuous upscale"),
        }
        // name is just a clarity alias for light_text.
        assert!(matches!(
            PreprocessParams::name(0.5, 4.0).upscale,
            Upscale::Continuous { .. }
        ));
        assert!(matches!(
            PreprocessParams::strip().upscale,
            Upscale::LineHeight
        ));
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
    fn dominant_text_band_finds_single_contiguous_run() {
        // One bright text run on rows 6..=12 (2 stroke pixels per row, clearing
        // min_ink=2 at this width); everything else dark. The band is that run.
        let (w, h) = (16usize, 24usize);
        let mut g = vec![0u8; w * h];
        for y in 6..=12 {
            g[y * w] = 255;
            g[y * w + 1] = 255;
        }
        assert_eq!(dominant_text_band(&g, w, h), Some((6, 12)));
    }

    #[test]
    fn dominant_text_band_keeps_brightest_run_and_drops_dim_noise() {
        // A dim noise band (value 100, below the 0.55*255≈140 ink level) on top,
        // a short bright run, and a taller bright run lower down. The tall run
        // carries the most stroke pixels, so it wins; the noise never counts.
        let (w, h) = (16usize, 30usize);
        let mut g = vec![0u8; w * h];
        for y in 0..=4 {
            for x in 0..w {
                g[y * w + x] = 100; // dim noise strip — no near-white pixels
            }
        }
        for y in 8..=9 {
            g[y * w] = 255;
            g[y * w + 1] = 255; // small bright run (energy 4)
        }
        for y in 18..=26 {
            g[y * w] = 255;
            g[y * w + 1] = 255; // tall bright run (energy 18) — wins
        }
        assert_eq!(dominant_text_band(&g, w, h), Some((18, 26)));
    }

    #[test]
    fn dominant_text_band_grows_across_thin_cap_and_descender_tips() {
        // Main run rows 10..=15 (2 px each). A single bright pixel one row above
        // (a cap/ascender tip) and one row below (a descender tail) fall below
        // min_ink but still carry a stroke pixel, so the band grows to include
        // them rather than clipping the glyph.
        let (w, h) = (16usize, 24usize);
        let mut g = vec![0u8; w * h];
        for y in 10..=15 {
            g[y * w] = 255;
            g[y * w + 1] = 255;
        }
        g[9 * w] = 255; // cap tip above
        g[16 * w] = 255; // descender below
        assert_eq!(dominant_text_band(&g, w, h), Some((9, 16)));
    }

    #[test]
    fn dominant_text_band_ignores_sub_min_ink_speckle() {
        // One bright pixel per row (below min_ink=2) is isolated sensor speckle,
        // not a stroke row: no run is ever seeded, so there is no band.
        let (w, h) = (16usize, 20usize);
        let mut g = vec![0u8; w * h];
        for y in 0..h {
            g[y * w] = 255;
        }
        assert_eq!(dominant_text_band(&g, w, h), None);
    }

    #[test]
    fn dominant_text_band_returns_none_for_blank_or_empty() {
        // All-dark crop has no peak; zero-sized crop has no rows.
        assert_eq!(dominant_text_band(&vec![0u8; 16 * 4], 16, 4), None);
        assert_eq!(dominant_text_band(&[], 0, 0), None);
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

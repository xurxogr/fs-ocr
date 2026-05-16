//! Tesseract OCR integration.
//!
//! Provides text extraction using Tesseract OCR with custom trained models.
//! Requires the leptess crate and system Tesseract installation.
//!
//! This module is only available when the `ocr-full` feature is enabled.

#![cfg(feature = "ocr-full")]

use std::ffi::CString;
use std::path::Path;
use std::sync::Mutex;

use leptess::tesseract::TessApi;

use crate::error::{FsOcrError, Result};
use crate::image_utils;

/// Create CString from a string, validating no null bytes.
/// Returns None if the string contains null bytes (which would be a security concern).
fn safe_cstring(s: &str) -> Option<CString> {
    CString::new(s).ok()
}

/// Configuration for Tesseract OCR.
#[derive(Debug, Clone)]
pub struct TesseractConfig {
    /// Path to tessdata directory.
    pub tessdata_path: String,
    /// Model name (e.g., "renner_numbers").
    pub model_name: String,
    /// Page segmentation mode (PSM).
    /// 6 = Assume uniform block of text
    /// 7 = Treat image as single text line
    pub psm: i32,
    /// Character whitelist (empty = all characters).
    pub whitelist: String,
}

impl TesseractConfig {
    /// Create config for quantity extraction.
    pub fn for_quantities(tessdata_path: &str) -> Self {
        Self {
            tessdata_path: tessdata_path.to_string(),
            model_name: "renner_numbers".to_string(),
            psm: 6, // Uniform block of text
            whitelist: "0123456789k+".to_string(),
        }
    }

    /// Create config for single-line text.
    pub fn for_text_line(tessdata_path: &str, model: &str) -> Self {
        Self {
            tessdata_path: tessdata_path.to_string(),
            model_name: model.to_string(),
            psm: 7, // Single text line
            whitelist: String::new(),
        }
    }

    /// Create config for text block.
    pub fn for_text_block(tessdata_path: &str, model: &str) -> Self {
        Self {
            tessdata_path: tessdata_path.to_string(),
            model_name: model.to_string(),
            psm: 6, // Uniform block of text
            whitelist: String::new(),
        }
    }
}

impl Default for TesseractConfig {
    fn default() -> Self {
        Self::for_quantities("tessdata")
    }
}

/// Text extractor using Tesseract OCR.
pub struct TextExtractor {
    /// Current configuration.
    config: TesseractConfig,
    /// Whether Tesseract is available.
    available: bool,
    /// Cached TessApi instance (thread-safe for PyO3 Sync requirement).
    api: Mutex<Option<TessApi>>,
}

impl TextExtractor {
    /// Create a new text extractor for quantities (numbers only).
    pub fn new<P: AsRef<Path>>(tessdata_path: P, model_name: &str) -> Result<Self> {
        let path = tessdata_path.as_ref();

        // Check if tessdata directory exists
        let available = path.exists();

        if !available {
            // Log warning but don't fail - allow stub behavior
            eprintln!(
                "Warning: Tessdata directory not found: {}. OCR will return empty results.",
                path.display()
            );
        }

        let config = TesseractConfig {
            tessdata_path: path.to_string_lossy().to_string(),
            model_name: model_name.to_string(),
            psm: 6,
            whitelist: "0123456789k+".to_string(),
        };

        Self::create_with_config(config, available)
    }

    /// Create a new text extractor for general text (no whitelist restriction).
    pub fn new_for_text<P: AsRef<Path>>(tessdata_path: P, model_name: &str) -> Result<Self> {
        let path = tessdata_path.as_ref();
        let available = path.exists();

        if !available {
            eprintln!(
                "Warning: Tessdata directory not found: {}. Text OCR will return empty results.",
                path.display()
            );
        }

        let config = TesseractConfig {
            tessdata_path: path.to_string_lossy().to_string(),
            model_name: model_name.to_string(),
            psm: 7,                   // Single line mode for type names
            whitelist: String::new(), // No whitelist - allow all characters
        };

        Self::create_with_config(config, available)
    }

    /// Create a new text extractor using system default tessdata directory (single line mode).
    pub fn new_for_text_default(model_name: &str) -> Result<Self> {
        let config = TesseractConfig {
            tessdata_path: String::new(), // Empty = use system default
            model_name: model_name.to_string(),
            psm: 7,                   // Single line mode for type names
            whitelist: String::new(), // No whitelist - allow all characters
        };

        Self::create_with_config_default(config)
    }

    /// Create a new text extractor for multi-line text using system default tessdata directory.
    pub fn new_for_text_block_default(model_name: &str) -> Result<Self> {
        let config = TesseractConfig {
            tessdata_path: String::new(), // Empty = use system default
            model_name: model_name.to_string(),
            psm: 6,                   // Block mode for multi-line text (shard region)
            whitelist: String::new(), // No whitelist - allow all characters
        };

        Self::create_with_config_default(config)
    }

    /// Internal helper to create extractor with system default tessdata.
    fn create_with_config_default(config: TesseractConfig) -> Result<Self> {
        // Pass None for tessdata_path to use system default
        let api = match TessApi::new(None, &config.model_name) {
            Ok(mut api) => {
                // Set PSM mode (static strings never fail)
                let psm_key =
                    CString::new("tessedit_pageseg_mode").expect("static string has no null bytes");
                let psm_val =
                    CString::new(config.psm.to_string()).expect("integer string has no null bytes");
                let _ = api.raw.set_variable(&psm_key, &psm_val);

                // Set whitelist if specified (validate no null bytes)
                if !config.whitelist.is_empty() {
                    let wl_key = CString::new("tessedit_char_whitelist")
                        .expect("static string has no null bytes");
                    if let Some(wl_val) = safe_cstring(config.whitelist.as_str()) {
                        let _ = api.raw.set_variable(&wl_key, &wl_val);
                    }
                    // Silently skip if whitelist contains null bytes (security)
                }

                Some(api)
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to initialize Tesseract with system tessdata: {:?}",
                    e
                );
                None
            }
        };

        Ok(Self {
            config,
            available: api.is_some(),
            api: Mutex::new(api),
        })
    }

    /// Internal helper to create extractor with config.
    fn create_with_config(config: TesseractConfig, available: bool) -> Result<Self> {
        // Pre-initialize TessApi if available
        let api = if available {
            match TessApi::new(Some(&config.tessdata_path), &config.model_name) {
                Ok(mut api) => {
                    // Set PSM mode (static strings never fail)
                    let psm_key = CString::new("tessedit_pageseg_mode")
                        .expect("static string has no null bytes");
                    let psm_val = CString::new(config.psm.to_string())
                        .expect("integer string has no null bytes");
                    let _ = api.raw.set_variable(&psm_key, &psm_val);

                    // Set whitelist (validate no null bytes)
                    if !config.whitelist.is_empty() {
                        let wl_key = CString::new("tessedit_char_whitelist")
                            .expect("static string has no null bytes");
                        if let Some(wl_val) = safe_cstring(config.whitelist.as_str()) {
                            let _ = api.raw.set_variable(&wl_key, &wl_val);
                        }
                        // Silently skip if whitelist contains null bytes (security)
                    }

                    Some(api)
                }
                Err(_) => None,
            }
        } else {
            None
        };

        Ok(Self {
            config,
            available,
            api: Mutex::new(api),
        })
    }

    /// Create with custom configuration.
    pub fn with_config(config: TesseractConfig) -> Result<Self> {
        let available = Path::new(&config.tessdata_path).exists();
        Ok(Self {
            config,
            available,
            api: Mutex::new(None),
        })
    }

    /// Extract text from an image.
    ///
    /// Args:
    ///     image: Grayscale or RGB image data
    ///     width: Image width
    ///     height: Image height
    ///     channels: Number of channels (1 for grayscale, 3 for RGB)
    ///
    /// Returns:
    ///     Extracted text string
    pub fn extract_text(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        channels: i32,
    ) -> Result<String> {
        if !self.available {
            return Ok(String::new());
        }

        // Use cached API or create new one
        let mut api_ref = self
            .api
            .lock()
            .map_err(|e| FsOcrError::Ocr(format!("Tesseract lock poisoned: {}", e)))?;
        let api = match api_ref.as_mut() {
            Some(api) => api,
            None => {
                // Create new API if not cached
                let new_api =
                    TessApi::new(Some(&self.config.tessdata_path), &self.config.model_name)
                        .map_err(|e| {
                            FsOcrError::Ocr(format!("Failed to initialize Tesseract: {}", e))
                        })?;
                *api_ref = Some(new_api);
                api_ref.as_mut().unwrap()
            }
        };

        // Set image data directly (no PNG encoding needed)
        let bytes_per_pixel = channels;
        let bytes_per_line = width * bytes_per_pixel;
        api.raw
            .set_image(image, width, height, bytes_per_pixel, bytes_per_line)
            .map_err(|e| FsOcrError::Ocr(format!("Failed to set image: {:?}", e)))?;

        // Set resolution to avoid warnings
        api.raw.set_source_resolution(72);

        // Get text
        let text = api
            .raw
            .get_utf8_text()
            .map_err(|e| FsOcrError::Ocr(format!("Failed to extract text: {:?}", e)))?;

        // Convert Text to String
        let text_str = text.as_ref().to_string_lossy();
        Ok(text_str.trim().to_string())
    }

    /// Extract quantities from an image.
    ///
    /// Uses the custom renner_numbers model optimized for game quantity display.
    ///
    /// Args:
    ///     image: Preprocessed binary image data
    ///     width: Image width
    ///     height: Image height
    ///
    /// Returns:
    ///     Nested vector of quantities (rows x columns)
    pub fn extract_quantities(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
    ) -> Result<Vec<Vec<i32>>> {
        // Extract text first
        let text = self.extract_text(image, width, height, 1)?;

        // Parse quantities from text
        Ok(super::quantity::parse_quantity_text(&text))
    }

    /// Extract text from a single line.
    pub fn extract_single_line(&self, image: &[u8], width: i32, height: i32) -> Result<String> {
        // Just use extract_text with grayscale
        self.extract_text(image, width, height, 1)
    }

    /// Get the tessdata path.
    pub fn tessdata_path(&self) -> &str {
        &self.config.tessdata_path
    }

    /// Get the model name.
    pub fn model_name(&self) -> &str {
        &self.config.model_name
    }

    /// Check if Tesseract is available.
    pub fn is_available(&self) -> bool {
        self.available
    }
}

impl Default for TextExtractor {
    fn default() -> Self {
        Self {
            config: TesseractConfig::default(),
            available: false,
            api: Mutex::new(None),
        }
    }
}

/// Preprocess an image for OCR.
///
/// Applies:
/// 1. Grayscale conversion (if needed)
/// 2. Upscaling (2x for better OCR accuracy)
/// 3. Binary thresholding (Otsu's method)
/// 4. Optional morphological operations
pub fn preprocess_for_ocr(
    image: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    upscale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Step 1: Convert to grayscale if needed
    let grayscale = if channels == 3 {
        image_utils::rgb_to_grayscale(image, width, height)
    } else {
        image.to_vec()
    };

    // Step 2: Upscale
    let new_width = (width as f64 * upscale_factor) as usize;
    let new_height = (height as f64 * upscale_factor) as usize;
    let upscaled = upscale_bilinear(&grayscale, width, height, new_width, new_height);

    // Step 3: Apply Otsu's threshold to create binary image
    let threshold = image_utils::compute_otsu_threshold(&upscaled);
    let binary = image_utils::apply_threshold(&upscaled, threshold);

    // Step 4: Invert (white text on black background -> black on white)
    let inverted: Vec<u8> = binary.iter().map(|&x| 255 - x).collect();

    (inverted, new_width, new_height)
}

/// Preprocess quantity composite image for OCR (Python-style).
///
/// Matches Python's stockpile_detector._build_quantity_composite_image:
/// 1. Upscale by 2/scale_factor
/// 2. Fixed threshold 120 with BINARY_INV
/// 3. Morphological close (2x2 kernel)
/// 4. Invert -> Erode -> Invert (thin text for better OCR)
pub fn preprocess_quantity_composite(
    image: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    scale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Default upscale: 2/scale_factor (matches Python)
    let upscale_factor = 2.0 / scale_factor;
    preprocess_quantity_with_upscale(image, width, height, channels, upscale_factor)
}

/// Preprocess quantity image with explicit upscale factor.
/// Use upscale_factor=1.0 for no upscaling.
pub fn preprocess_quantity_with_upscale(
    image: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    upscale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Step 1: Convert to grayscale if needed
    let grayscale = if channels == 3 {
        image_utils::rgb_to_grayscale(image, width, height)
    } else {
        image.to_vec()
    };

    // Step 2: Upscale (or skip if factor is 1.0)
    let (processed, new_width, new_height) = if (upscale_factor - 1.0).abs() < 0.01 {
        // No upscale needed
        (grayscale, width, height)
    } else {
        let new_w = (width as f64 * upscale_factor) as usize;
        let new_h = (height as f64 * upscale_factor) as usize;
        let upscaled = upscale_bilinear(&grayscale, width, height, new_w, new_h);
        (upscaled, new_w, new_h)
    };

    // Step 3: Fixed threshold 120 with BINARY_INV (pixels < 120 become 255)
    let binary: Vec<u8> = processed
        .iter()
        .map(|&x| if x < 120 { 255 } else { 0 })
        .collect();

    // Step 4: Morphological close (dilate then erode) with 2x2 kernel
    let dilated = dilate_2x2(&binary, new_width, new_height);
    let closed = erode_2x2(&dilated, new_width, new_height);

    // Step 5: Invert -> Erode -> Invert (thin text)
    let inverted: Vec<u8> = closed.iter().map(|&x| 255 - x).collect();
    let eroded = erode_2x2(&inverted, new_width, new_height);
    let final_img: Vec<u8> = eroded.iter().map(|&x| 255 - x).collect();

    (final_img, new_width, new_height)
}

/// Dilate with 2x2 kernel (max filter).
fn dilate_2x2(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut max_val = 0u8;
            for dy in 0..2 {
                for dx in 0..2 {
                    let ny = (y + dy).min(height - 1);
                    let nx = (x + dx).min(width - 1);
                    max_val = max_val.max(image[ny * width + nx]);
                }
            }
            result[y * width + x] = max_val;
        }
    }
    result
}

/// Erode with 2x2 kernel (min filter).
fn erode_2x2(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut min_val = 255u8;
            for dy in 0..2 {
                for dx in 0..2 {
                    let ny = (y + dy).min(height - 1);
                    let nx = (x + dx).min(width - 1);
                    min_val = min_val.min(image[ny * width + nx]);
                }
            }
            result[y * width + x] = min_val;
        }
    }
    result
}

/// Bilinear upscaling.
fn upscale_bilinear(
    image: &[u8],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; dst_width * dst_height];

    let x_ratio = src_width as f64 / dst_width as f64;
    let y_ratio = src_height as f64 / dst_height as f64;

    for y in 0..dst_height {
        for x in 0..dst_width {
            let src_x = x as f64 * x_ratio;
            let src_y = y as f64 * y_ratio;

            let x0 = src_x.floor() as usize;
            let y0 = src_y.floor() as usize;
            let x1 = (x0 + 1).min(src_width - 1);
            let y1 = (y0 + 1).min(src_height - 1);

            let x_diff = src_x - x0 as f64;
            let y_diff = src_y - y0 as f64;

            let p00 = image[y0 * src_width + x0] as f64;
            let p10 = image[y0 * src_width + x1] as f64;
            let p01 = image[y1 * src_width + x0] as f64;
            let p11 = image[y1 * src_width + x1] as f64;

            let value = p00 * (1.0 - x_diff) * (1.0 - y_diff)
                + p10 * x_diff * (1.0 - y_diff)
                + p01 * (1.0 - x_diff) * y_diff
                + p11 * x_diff * y_diff;

            result[y * dst_width + x] = value.round() as u8;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_extractor() {
        let extractor = TextExtractor::default();
        assert_eq!(extractor.model_name(), "renner_numbers");
        assert!(!extractor.is_available()); // Default has no valid path
    }

    #[test]
    fn test_tesseract_config() {
        let config = TesseractConfig::for_quantities("./tessdata");
        assert_eq!(config.model_name, "renner_numbers");
        assert_eq!(config.whitelist, "0123456789k+");
        assert_eq!(config.psm, 6);
    }

    #[test]
    fn test_rgb_to_grayscale() {
        // All red pixels
        let rgb = vec![255u8, 0, 0, 255, 0, 0];
        let gray = image_utils::rgb_to_grayscale(&rgb, 2, 1);
        assert_eq!(gray.len(), 2);
        // Red converts to ~76 gray (0.299 * 255)
        assert!(gray[0] > 70 && gray[0] < 80);
    }

    #[test]
    fn test_otsu_threshold() {
        // Create bimodal histogram (half black, half white)
        let mut image = vec![0u8; 100];
        for i in 50..100 {
            image[i] = 255;
        }

        let threshold = image_utils::compute_otsu_threshold(&image);
        let binary = image_utils::apply_threshold(&image, threshold);

        // Should split roughly in the middle
        let white_count = binary.iter().filter(|&&x| x == 255).count();
        assert!(white_count > 40 && white_count < 60);
    }

    #[test]
    fn test_preprocess_for_ocr() {
        let rgb = vec![128u8; 10 * 10 * 3];
        let (processed, new_w, new_h) = preprocess_for_ocr(&rgb, 10, 10, 3, 2.0);

        assert_eq!(new_w, 20);
        assert_eq!(new_h, 20);
        assert_eq!(processed.len(), 20 * 20);
    }

    #[test]
    fn test_upscale_bilinear() {
        // Simple 2x2 image
        let image = vec![0u8, 255, 255, 0];
        let upscaled = upscale_bilinear(&image, 2, 2, 4, 4);

        assert_eq!(upscaled.len(), 16);
        // Corner values should be preserved
        assert_eq!(upscaled[0], 0);
        assert_eq!(upscaled[3], 255);
    }
}

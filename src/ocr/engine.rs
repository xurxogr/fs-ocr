//! OCR engine abstraction.
//!
//! Provides a trait-based interface for OCR backends.

use crate::error::Result;

/// OCR engine configuration.
#[derive(Debug, Clone)]
pub struct OcrConfig {
    /// Path to model/data directory.
    pub data_path: String,
    /// Model name (e.g., "renner_numbers", "eng").
    pub model_name: String,
    /// Page segmentation mode.
    /// 6 = Assume uniform block of text
    /// 7 = Treat image as single text line
    pub psm: i32,
    /// Character whitelist (empty = all characters).
    pub whitelist: String,
    /// Decode-time character mask for the ocrs backend. When set, the recognizer
    /// may only emit these characters (others are excluded before CTC decode),
    /// which keeps closed-vocabulary fields on-script. `None` = no restriction.
    /// Ignored by the Tesseract backend.
    pub allowed_chars: Option<String>,
}

impl OcrConfig {
    /// Create config for quantity extraction (digits only).
    pub fn for_quantities(data_path: &str) -> Self {
        Self {
            data_path: data_path.to_string(),
            model_name: "renner_numbers".to_string(),
            psm: 6,
            whitelist: "0123456789k+".to_string(),
            allowed_chars: None,
        }
    }

    /// Create config for single-line text.
    pub fn for_text_line(data_path: &str, model: &str) -> Self {
        Self {
            data_path: data_path.to_string(),
            model_name: model.to_string(),
            psm: 7,
            whitelist: String::new(),
            allowed_chars: None,
        }
    }

    /// Create config for text block (multi-line).
    pub fn for_text_block(data_path: &str, model: &str) -> Self {
        Self {
            data_path: data_path.to_string(),
            model_name: model.to_string(),
            psm: 6,
            whitelist: String::new(),
            allowed_chars: None,
        }
    }
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self::for_quantities("models")
    }
}

/// OCR engine trait for text extraction.
///
/// Implementations must be Send + Sync to support multi-threaded usage.
pub trait OcrEngine: Send + Sync {
    /// Extract text from a grayscale image.
    fn extract_text(&self, image: &[u8], width: i32, height: i32) -> Result<String>;

    /// Check if this OCR engine is available and initialized.
    fn is_available(&self) -> bool;

    /// Check if this engine supports multilingual text (Chinese, Russian, etc.).
    fn supports_multilingual(&self) -> bool;

    /// Get a description of this OCR engine for debugging.
    fn engine_name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ocr_config_defaults() {
        let config = OcrConfig::default();
        assert_eq!(config.model_name, "renner_numbers");
        assert_eq!(config.whitelist, "0123456789k+");
        assert_eq!(config.psm, 6);
    }

    #[test]
    fn test_ocr_config_for_text() {
        let config = OcrConfig::for_text_line("data", "eng");
        assert_eq!(config.model_name, "eng");
        assert_eq!(config.data_path, "data");
        assert!(config.whitelist.is_empty());
        assert_eq!(config.psm, 7);
    }
}

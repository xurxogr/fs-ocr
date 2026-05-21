//! OCR components for text and quantity extraction.
//!
//! This module provides OCR functionality with two backends:
//! - **ocrs (default)**: Pure Rust, Latin characters and digits, no external deps
//! - **Tesseract (ocr-full feature)**: Multilingual support, requires system Tesseract
//!
//! Build with `--features ocr-full` to enable Tesseract backend.

pub mod basic;
pub mod digit_matcher;
pub mod engine;
pub mod preprocess;
pub mod quantity;

// Tesseract backend (requires system Tesseract installation)
#[cfg(feature = "ocr-full")]
pub mod tesseract;

// Re-exports
pub use basic::OcrsEngine;
pub use engine::{OcrConfig, OcrEngine};
pub use preprocess::{
    preprocess_for_ocr, preprocess_quantity_composite, preprocess_quantity_with_upscale,
};
pub use quantity::parse_quantity;

// TextExtractor: Use Tesseract when ocr-full is enabled, otherwise use ocrs wrapper
#[cfg(feature = "ocr-full")]
pub use tesseract::TextExtractor;

#[cfg(not(feature = "ocr-full"))]
pub use self::ocrs_extractor::TextExtractor;

/// Create an OCR engine for quantities (digits).
pub fn create_quantity_engine(data_path: &str) -> Option<Box<dyn OcrEngine>> {
    let config = OcrConfig::for_quantities(data_path);
    match OcrsEngine::new(config) {
        Ok(engine) if engine.is_available() => Some(Box::new(engine)),
        _ => None,
    }
}

/// Create an OCR engine for single-line text.
pub fn create_text_engine(data_path: &str, _model: &str) -> Option<Box<dyn OcrEngine>> {
    let config = OcrConfig::for_text_line(data_path, "eng");
    match OcrsEngine::new(config) {
        Ok(engine) if engine.is_available() => Some(Box::new(engine)),
        _ => None,
    }
}

/// Create an OCR engine for multi-line text blocks.
pub fn create_block_engine(data_path: &str, _model: &str) -> Option<Box<dyn OcrEngine>> {
    let config = OcrConfig::for_text_block(data_path, "eng");
    match OcrsEngine::new(config) {
        Ok(engine) if engine.is_available() => Some(Box::new(engine)),
        _ => None,
    }
}

/// Ocrs-based TextExtractor wrapper (used when ocr-full is not enabled).
#[cfg(not(feature = "ocr-full"))]
mod ocrs_extractor {
    use super::{quantity, OcrConfig, OcrEngine, OcrsEngine};

    pub struct TextExtractor {
        engine: Option<OcrsEngine>,
        model_name: String,
        data_path: String,
    }

    impl TextExtractor {
        /// Create a new text extractor for quantities.
        pub fn new<P: AsRef<std::path::Path>>(
            data_path: P,
            model_name: &str,
        ) -> crate::error::Result<Self> {
            let path = data_path.as_ref().to_string_lossy().to_string();
            let config = OcrConfig {
                data_path: path.clone(),
                model_name: model_name.to_string(),
                psm: 6,
                whitelist: "0123456789k+".to_string(),
            };
            let engine = OcrsEngine::new(config).ok();
            Ok(Self {
                engine,
                model_name: model_name.to_string(),
                data_path: path,
            })
        }

        /// Create for text (no whitelist).
        pub fn new_for_text<P: AsRef<std::path::Path>>(
            data_path: P,
            model_name: &str,
        ) -> crate::error::Result<Self> {
            let path = data_path.as_ref().to_string_lossy().to_string();
            let config = OcrConfig::for_text_line(&path, model_name);
            let engine = OcrsEngine::new(config).ok();
            Ok(Self {
                engine,
                model_name: model_name.to_string(),
                data_path: path,
            })
        }

        /// Create using system default data path.
        pub fn new_for_text_default(model_name: &str) -> crate::error::Result<Self> {
            // Use "data" as default path for ocrs models
            Self::new_for_text("data", model_name)
        }

        /// Create for multi-line text.
        pub fn new_for_text_block_default(model_name: &str) -> crate::error::Result<Self> {
            // Use "data" as default path for ocrs models
            let config = OcrConfig::for_text_block("data", model_name);
            let engine = OcrsEngine::new(config).ok();
            Ok(Self {
                engine,
                model_name: model_name.to_string(),
                data_path: "data".to_string(),
            })
        }

        /// Extract text from image.
        pub fn extract_text(
            &self,
            image: &[u8],
            width: i32,
            height: i32,
            _channels: i32,
        ) -> crate::error::Result<String> {
            match &self.engine {
                Some(engine) => engine.extract_text(image, width, height),
                None => Ok(String::new()),
            }
        }

        /// Extract quantities from image.
        pub fn extract_quantities(
            &self,
            image: &[u8],
            width: i32,
            height: i32,
        ) -> crate::error::Result<Vec<Vec<i32>>> {
            let text = self.extract_text(image, width, height, 1)?;
            Ok(quantity::parse_quantity_text(&text))
        }

        /// Check if OCR is available.
        pub fn is_available(&self) -> bool {
            self.engine
                .as_ref()
                .map(|e| e.is_available())
                .unwrap_or(false)
        }

        /// Get the data path.
        pub fn data_path(&self) -> &str {
            &self.data_path
        }

        /// Get the model name.
        pub fn model_name(&self) -> &str {
            &self.model_name
        }
    }

    impl Default for TextExtractor {
        fn default() -> Self {
            Self {
                engine: None,
                model_name: "renner_numbers".to_string(),
                data_path: String::new(),
            }
        }
    }
}

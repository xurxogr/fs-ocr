//! OCR components for text and quantity extraction.
//!
//! Text is recognized by the pure-Rust **ocrs** backend (Latin/Cyrillic digits
//! and letters, no external dependencies) — the single OCR path for type, shard,
//! timestamp and custom names.
//!
//! Chinese *custom* stockpile names fall outside the ocrs alphabet; they are read
//! by an optional runtime call to the system `tesseract` CLI (see
//! [`ChineseNameReader`]). When that binary isn't installed the name is left
//! unread and everything else still scans.

pub mod digit_matcher;
pub mod engine;
pub mod preprocess;
pub mod quantity;

// Pure-Rust ocrs backend: the single OCR path for type, shard, timestamp and
// Latin/Cyrillic names.
pub mod basic;

// Optional Chinese custom-name reader via the system `tesseract` CLI (runtime
// dependency; a no-op when the binary isn't installed).
pub mod tesseract;

// Re-exports
pub use basic::OcrsEngine;
pub use engine::{OcrConfig, OcrEngine};
pub use preprocess::{
    preprocess_for_ocr, preprocess_quantity_composite, preprocess_quantity_with_upscale,
};
pub use quantity::parse_quantity;
pub use tesseract::ChineseNameReader;

pub use self::ocrs_extractor::TextExtractor;

/// Ocrs-based single-line/-block text extractor used across the pipeline.
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
                allowed_chars: None,
            };
            let engine = OcrsEngine::new(config).ok();
            Ok(Self {
                engine,
                model_name: model_name.to_string(),
                data_path: path,
            })
        }

        /// Create a single-line text extractor restricted to `allowed_chars`.
        /// The ocrs recognizer may then only emit those characters, keeping
        /// closed-vocabulary fields (shard names, the localized timestamp line)
        /// on-script. Mirrors `new_for_text_block_default`'s hardcoded "data"
        /// path so the masked engine loads the same recognition model.
        pub fn new_for_text_default_with_allowed(
            model_name: &str,
            allowed_chars: &str,
        ) -> crate::error::Result<Self> {
            let mut config = OcrConfig::for_text_line("data", model_name);
            config.allowed_chars = Some(allowed_chars.to_string());
            let engine = OcrsEngine::new(config).ok();
            Ok(Self {
                engine,
                model_name: model_name.to_string(),
                data_path: "data".to_string(),
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

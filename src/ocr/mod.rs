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
pub use tesseract::ChineseNameReader;

pub use self::ocrs_extractor::TextExtractor;

/// Ocrs-based single-line/-block text extractor used across the pipeline.
mod ocrs_extractor {
    use super::{OcrConfig, OcrEngine, OcrsEngine};

    #[derive(Default)]
    pub struct TextExtractor {
        engine: Option<OcrsEngine>,
    }

    impl TextExtractor {
        /// Create a single-line text extractor restricted to `allowed_chars`.
        /// The ocrs recognizer may then only emit those characters, keeping
        /// closed-vocabulary fields (shard names, the localized timestamp line)
        /// on-script.
        pub fn new_for_text_default_with_allowed(
            _model_name: &str,
            allowed_chars: &str,
        ) -> crate::error::Result<Self> {
            let mut config = OcrConfig::for_text_line("data");
            config.allowed_chars = Some(allowed_chars.to_string());
            let engine = OcrsEngine::new(config).ok();
            Ok(Self { engine })
        }

        /// Create for text (no whitelist).
        pub fn new_for_text<P: AsRef<std::path::Path>>(
            data_path: P,
            _model_name: &str,
        ) -> crate::error::Result<Self> {
            let path = data_path.as_ref().to_string_lossy().to_string();
            let config = OcrConfig::for_text_line(&path);
            let engine = OcrsEngine::new(config).ok();
            Ok(Self { engine })
        }

        /// Create using system default data path.
        pub fn new_for_text_default(model_name: &str) -> crate::error::Result<Self> {
            // Use "data" as default path for ocrs models
            Self::new_for_text("data", model_name)
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
    }
}

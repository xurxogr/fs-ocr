//! Basic OCR engine using ocrs (pure Rust).
//!
//! Provides text extraction using the ocrs crate for pure Rust OCR.
//! Supports Latin characters and digits only.

use std::sync::Mutex;

use ocrs::{ImageSource, OcrEngine as OcrsOcrEngine, OcrEngineParams};
use rten::Model;

use crate::error::{FsOcrError, Result};

use super::engine::{OcrConfig, OcrEngine};

/// Path to the detection model (relative to data directory).
const DETECTION_MODEL: &str = "text-detection.rten";
/// Path to the recognition model (relative to data directory).
const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// Ocrs-based OCR engine implementing the OcrEngine trait.
pub struct OcrsEngine {
    /// Configuration.
    config: OcrConfig,
    /// Whether the engine is available.
    available: bool,
    /// Cached OCR engine instance (thread-safe).
    engine: Mutex<Option<OcrsOcrEngine>>,
}

impl OcrsEngine {
    /// Create a new ocrs engine with the given configuration.
    pub fn new(config: OcrConfig) -> Result<Self> {
        // Try to load models
        let (engine, available) = Self::try_load_engine(&config.data_path);

        if !available {
            eprintln!(
                "Warning: ocrs models not found in '{}'. Basic OCR will return empty results.",
                config.data_path
            );
            eprintln!(
                "Expected models: {} and {}",
                DETECTION_MODEL, RECOGNITION_MODEL
            );
        }

        Ok(Self {
            config,
            available,
            engine: Mutex::new(engine),
        })
    }

    /// Try to load the ocrs engine from the data path.
    fn try_load_engine(data_path: &str) -> (Option<OcrsOcrEngine>, bool) {
        let base_path = std::path::Path::new(data_path);
        let detection_path = base_path.join(DETECTION_MODEL);
        let recognition_path = base_path.join(RECOGNITION_MODEL);

        // Check if both model files exist
        if !detection_path.exists() || !recognition_path.exists() {
            return (None, false);
        }

        // Load models
        let detection_model = match Model::load_file(&detection_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to load detection model: {:?}", e);
                return (None, false);
            }
        };

        let recognition_model = match Model::load_file(&recognition_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to load recognition model: {:?}", e);
                return (None, false);
            }
        };

        // Create OCR engine
        match OcrsOcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        }) {
            Ok(engine) => (Some(engine), true),
            Err(e) => {
                eprintln!("Failed to create ocrs engine: {:?}", e);
                (None, false)
            }
        }
    }

    /// Ensure the engine is initialized.
    fn ensure_engine(&self) -> Result<()> {
        if !self.available {
            return Ok(()); // Nothing to initialize
        }

        let mut engine_guard = self
            .engine
            .lock()
            .map_err(|e| FsOcrError::Ocr(format!("Engine lock poisoned: {}", e)))?;

        if engine_guard.is_none() {
            let (engine, _) = Self::try_load_engine(&self.config.data_path);
            *engine_guard = engine;
        }

        Ok(())
    }
}

impl OcrEngine for OcrsEngine {
    fn extract_text(&self, image: &[u8], width: i32, height: i32) -> Result<String> {
        if !self.available {
            return Ok(String::new());
        }

        self.ensure_engine()?;

        let engine_guard = self
            .engine
            .lock()
            .map_err(|e| FsOcrError::Ocr(format!("Engine lock poisoned: {}", e)))?;

        let engine = match engine_guard.as_ref() {
            Some(e) => e,
            None => return Ok(String::new()),
        };

        // Convert grayscale to RGBA format expected by ocrs
        let rgba: Vec<u8> = image
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect();

        // Create ImageSource and prepare input
        let img_source = match ImageSource::from_bytes(&rgba, (width as u32, height as u32)) {
            Ok(src) => src,
            Err(e) => {
                return Err(FsOcrError::Ocr(format!(
                    "Failed to create image source: {:?}",
                    e
                )));
            }
        };

        let input = match engine.prepare_input(img_source) {
            Ok(i) => i,
            Err(e) => {
                return Err(FsOcrError::Ocr(format!(
                    "Failed to prepare image for OCR: {:?}",
                    e
                )));
            }
        };

        // Detect text
        let word_rects = match engine.detect_words(&input) {
            Ok(w) => w,
            Err(e) => {
                return Err(FsOcrError::Ocr(format!(
                    "Failed to detect text regions: {:?}",
                    e
                )));
            }
        };

        // Recognize text from detected regions
        let line_rects = engine.find_text_lines(&input, &word_rects);
        let text = match engine.recognize_text(&input, &line_rects) {
            Ok(lines) => lines
                .iter()
                .filter_map(|line| line.as_ref().map(|l| l.to_string()))
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => {
                return Err(FsOcrError::Ocr(format!(
                    "Failed to recognize text: {:?}",
                    e
                )));
            }
        };

        // Apply whitelist filter if configured
        let filtered = if self.config.whitelist.is_empty() {
            text
        } else {
            text.chars()
                .filter(|c| c.is_whitespace() || self.config.whitelist.contains(*c))
                .collect()
        };

        Ok(filtered.trim().to_string())
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn supports_multilingual(&self) -> bool {
        // ocrs only supports Latin script
        false
    }

    fn engine_name(&self) -> &'static str {
        "ocrs"
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ocrs_engine_unavailable() {
        let config = OcrConfig::for_quantities("nonexistent_path");
        let engine = OcrsEngine::new(config).unwrap();
        assert!(!engine.is_available());
        assert!(!engine.supports_multilingual());
        assert_eq!(engine.engine_name(), "ocrs");
    }

    #[test]
    fn test_extract_text_when_unavailable() {
        let config = OcrConfig::for_quantities("nonexistent_path");
        let engine = OcrsEngine::new(config).unwrap();
        let result = engine.extract_text(&[128; 100], 10, 10).unwrap();
        assert!(result.is_empty());
    }
}

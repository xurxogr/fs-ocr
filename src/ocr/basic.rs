//! Basic OCR engine using ocrs (pure Rust).
//!
//! Provides text extraction using the ocrs crate for pure Rust OCR.
//! Supports Latin characters and digits only.
//!
//! Uses recognition-only mode (no detection) for faster processing
//! since we already know the text regions from detection.

use std::sync::Mutex;

use ocrs::{ImageSource, OcrEngine as OcrsOcrEngine, OcrEngineParams};
use rten::Model;
use rten_imageproc::{RectF, RotatedRect};

use crate::error::{FsOcrError, Result};

use super::engine::{OcrConfig, OcrEngine};

/// Filename of an optional user-supplied recognition model in the data
/// directory. When present it overrides the embedded model below.
/// Note: We don't use detection model - we already know where text is.
const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// Recognition model compiled into the binary so the library and CLI work with
/// no external data files. A file at `<data_path>/text-recognition.rten` takes
/// precedence, letting users swap in a better model without recompiling.
static EMBEDDED_RECOGNITION_MODEL: &[u8] = include_bytes!("../../data/text-recognition.rten");

/// Alphabet for the recognition model. MUST match `training/alphabet.txt` and
/// the model's output-column order exactly (class `i+1` ↔ char at byte-index
/// `i`; class 0 is the CTC blank). 186 chars: ocrs' default Latin/digit/symbol
/// set + full Russian Cyrillic + the closed Hanzi set from the stockpile-type
/// translations + the CN timestamp glyphs (日时分，). Regenerate via
/// `training/build_alphabet.py` if the type translations change.
const RECOGNITION_ALPHABET: &str = r##" 0123456789!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~EABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyzАБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя营地要塞安全屋遗迹基堡边境城镇下仓库海港日时分，"##;

/// Minimum pixel spread (max − min) for a region to be considered to contain
/// ink. Uniform regions below this carry no text, so recognition is skipped to
/// stop the model hallucinating glyphs from contrast-free pixels.
const MIN_CONTRAST: u8 = 8;

/// Whether an image region has too little contrast to contain text.
fn is_blank(image: &[u8]) -> bool {
    match (image.iter().min(), image.iter().max()) {
        (Some(&lo), Some(&hi)) => hi - lo < MIN_CONTRAST,
        _ => true, // empty buffer
    }
}

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
        // Set RTEN_NUM_THREADS=1 for small image OCR performance.
        // For small images like text regions (~100x20 pixels), single-threaded
        // execution is 10x faster due to avoiding thread coordination overhead.
        // SAFETY: This is safe to call before model loading. Environment variable
        // access is inherently racy but rten reads it once at model load time.
        unsafe {
            std::env::set_var("RTEN_NUM_THREADS", "1");
        }

        // Try to load models
        let (engine, available) =
            Self::try_load_engine(&config.data_path, config.allowed_chars.as_deref());

        if !available {
            eprintln!("Warning: failed to load the ocrs recognition model. Basic OCR will return empty results.");
        }

        Ok(Self {
            config,
            available,
            engine: Mutex::new(engine),
        })
    }

    /// Try to load the ocrs engine.
    /// Prefers a user-supplied model file in the data path, falling back to the
    /// embedded model. Only loads recognition (no detection) for speed.
    fn try_load_engine(
        data_path: &str,
        allowed_chars: Option<&str>,
    ) -> (Option<OcrsOcrEngine>, bool) {
        let recognition_path = std::path::Path::new(data_path).join(RECOGNITION_MODEL);

        // A model file in the data dir overrides the embedded one; otherwise
        // fall back to the model compiled into the binary.
        let recognition_model = if recognition_path.exists() {
            match Model::load_file(&recognition_path) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Failed to load recognition model: {:?}", e);
                    return (None, false);
                }
            }
        } else {
            match Model::load_static_slice(EMBEDDED_RECOGNITION_MODEL) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Failed to load embedded recognition model: {:?}", e);
                    return (None, false);
                }
            }
        };

        // Create OCR engine with recognition only
        match OcrsOcrEngine::new(OcrEngineParams {
            recognition_model: Some(recognition_model),
            alphabet: Some(RECOGNITION_ALPHABET.to_string()),
            allowed_chars: allowed_chars.map(|s| s.to_string()),
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
            let (engine, _) =
                Self::try_load_engine(&self.config.data_path, self.config.allowed_chars.as_deref());
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

        // A region with no contrast carries no text; skip recognition so the
        // model can't hallucinate glyphs from uniform pixels.
        if is_blank(image) {
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

        // Create ImageSource and prepare input
        let img_source = match ImageSource::from_bytes(image, (width as u32, height as u32)) {
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

        // Use direct recognition with known bounding rect (skip detection)
        // The entire image is the text region
        let rect = RotatedRect::from_rect(RectF::from_tlhw(0.0, 0.0, height as f32, width as f32));

        // Perform text recognition
        let text_lines = match engine.recognize_text(&input, &[[rect].to_vec()]) {
            Ok(lines) => lines,
            Err(e) => {
                return Err(FsOcrError::Ocr(format!("OCR recognition failed: {:?}", e)));
            }
        };

        // Join all recognized lines
        let text: String = text_lines
            .into_iter()
            .flatten()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("");

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_text_blank_returns_empty() {
        let config = OcrConfig::for_quantities("nonexistent_path");
        let engine = OcrsEngine::new(config).unwrap();
        let result = engine.extract_text(&[128; 100], 10, 10).unwrap();
        assert!(result.is_empty());
    }
}

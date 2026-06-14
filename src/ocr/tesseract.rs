//! Optional Chinese stockpile-name reader via the system `tesseract` CLI.
//!
//! The pure-Rust ocrs recognizer covers Latin and Cyrillic, but its alphabet
//! only carries the closed Hanzi set used by the stockpile *type* labels — it
//! cannot read arbitrary Chinese *custom* names. Those are read here by shelling
//! out to the system `tesseract` binary, if one is installed.
//!
//! This is a pure runtime dependency: there is no compile-time link against
//! Tesseract/Leptonica. When the binary (or its `chi_sim` language data) is not
//! present, [`ChineseNameReader::read`] returns `None` and the Chinese custom
//! name is simply left unread — every other field still scans normally.
//!
//! To enable Chinese custom names, a consumer installs Tesseract and the
//! simplified-Chinese data, e.g. `apt install tesseract-ocr tesseract-ocr-chi-sim`
//! or `brew install tesseract tesseract-lang`. The binary and language can be
//! overridden via the `FS_OCR_TESSERACT` and `FS_OCR_TESSERACT_LANG` env vars.

use std::io::Write;
use std::process::{Command, Stdio};

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

/// Env var overriding the `tesseract` binary path (default: `tesseract`).
const ENV_BIN: &str = "FS_OCR_TESSERACT";
/// Env var overriding the OCR language (default: `chi_sim`).
const ENV_LANG: &str = "FS_OCR_TESSERACT_LANG";
const DEFAULT_BIN: &str = "tesseract";
const DEFAULT_LANG: &str = "chi_sim";

/// Reads Chinese stockpile names by invoking the system `tesseract` CLI.
///
/// Constructed once and cached; construction probes for the binary so the cost
/// of a missing install is paid a single time.
pub struct ChineseNameReader {
    bin: String,
    lang: String,
    available: bool,
}

impl ChineseNameReader {
    /// Probe for the `tesseract` binary and build a reader.
    ///
    /// `available` reflects only whether the binary can be executed; a missing
    /// `chi_sim` language pack surfaces later as an empty/failed read (so the
    /// name is left unread) rather than an error.
    pub fn new() -> Self {
        let bin = std::env::var(ENV_BIN).unwrap_or_else(|_| DEFAULT_BIN.to_string());
        let lang = std::env::var(ENV_LANG).unwrap_or_else(|_| DEFAULT_LANG.to_string());

        let available = Command::new(&bin)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        Self {
            bin,
            lang,
            available,
        }
    }

    /// Whether the `tesseract` binary was found.
    pub fn is_available(&self) -> bool {
        self.available
    }

    /// Read a single line of Chinese text from a grayscale name crop.
    ///
    /// `gray` is the recognizer-preprocessed buffer (light text on a dark
    /// background, ocrs polarity); it is inverted to the dark-on-light polarity
    /// Tesseract expects before encoding. Returns the trimmed text, or `None`
    /// when Tesseract is unavailable, fails, or yields nothing.
    pub fn read(&self, gray: &[u8], width: u32, height: u32) -> Option<String> {
        if !self.available || gray.len() != (width as usize) * (height as usize) || gray.is_empty()
        {
            return None;
        }

        // ocrs polarity is light-on-dark; Tesseract reads dark-on-light best.
        let inverted: Vec<u8> = gray.iter().map(|&p| 255 - p).collect();

        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&inverted, width, height, ExtendedColorType::L8)
            .ok()?;

        let text = self.run_tesseract(png)?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Run `tesseract - stdout -l <lang> --psm 7`, piping the PNG over stdin.
    ///
    /// stdin is written from a worker thread so a PNG larger than the pipe
    /// buffer can't deadlock against Tesseract not having started reading.
    fn run_tesseract(&self, png: Vec<u8>) -> Option<String> {
        let mut child = Command::new(&self.bin)
            .args(["-", "stdout", "-l", &self.lang, "--psm", "7"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let mut stdin = child.stdin.take()?;
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&png);
            // stdin drops here, signaling EOF to Tesseract.
        });

        let output = child.wait_with_output().ok()?;
        let _ = writer.join();

        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Default for ChineseNameReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_rejects_mismatched_buffer() {
        let reader = ChineseNameReader {
            bin: DEFAULT_BIN.to_string(),
            lang: DEFAULT_LANG.to_string(),
            available: true,
        };
        // Buffer length doesn't match width*height -> None without spawning.
        assert!(reader.read(&[0u8; 10], 4, 4).is_none());
    }

    #[test]
    fn unavailable_reader_reads_none() {
        let reader = ChineseNameReader {
            bin: "definitely-not-a-real-binary-xyz".to_string(),
            lang: DEFAULT_LANG.to_string(),
            available: false,
        };
        assert!(!reader.is_available());
        assert!(reader.read(&[0u8; 16], 4, 4).is_none());
    }
}

//! fs-ocr: Fast OCR library for Foxhole stockpile screenshots.
//!
//! This library provides high-performance OCR and template matching
//! for extracting item data from Foxhole game screenshots.
//!
//! # Python Usage
//!
//! ```python
//! from fs_ocr import StockpileScanner, ScanConfig
//!
//! scanner = StockpileScanner(database_path="templates.h5")
//! result = scanner.scan(image, faction="wardens")
//! print(result.to_json())
//!
//! # With optional config
//! config = ScanConfig(confidence_gap=0.02)
//! result = scanner.scan(image, config=config)
//! ```

#[cfg(feature = "python")]
use std::path::Path;

#[cfg(feature = "python")]
use numpy::{PyReadonlyArray3, PyUntypedArrayMethods};
#[cfg(feature = "python")]
use pyo3::prelude::*;

// Public API surface. Keep this minimal and intentional: only the types a
// consumer needs to drive a scan and read its result. Everything else is a
// crate-internal implementation detail (see the `mod` declarations below).
pub mod config; // ScanConfig
pub mod coordinator; // ScanPipeline
pub mod enums; // ItemFaction, ItemCategory, StockpileType, GameLanguage
pub mod error; // FsOcrError, Result (returned by the public API)
pub mod models; // Stockpile and friends (scan result types)

// Crate-internal implementation modules — reachable via `crate::` paths but not
// part of the published API.
mod constants;
mod detector;
mod image_utils;
mod ocr;
mod template;
mod text_utils;

#[cfg(feature = "python")]
use config::ScanConfig;
#[cfg(feature = "python")]
use coordinator::ScanPipeline;
#[cfg(feature = "python")]
use enums::{ItemCategory, ItemFaction, StockpileType};
#[cfg(feature = "python")]
use models::{ItemCandidate, Stockpile, StockpileItem, Timing};

/// Allowed extensions for database files.
#[cfg(feature = "python")]
const ALLOWED_DB_EXTENSIONS: &[&str] = &["h5", "hdf5"];

/// Allowed extensions for image files.
#[cfg(feature = "python")]
const ALLOWED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp", "gif", "webp", "tiff"];

/// Validate database path has allowed extension.
#[cfg(feature = "python")]
fn validate_database_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);

    // Check extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext {
        Some(ref e) if ALLOWED_DB_EXTENSIONS.contains(&e.as_str()) => Ok(()),
        Some(e) => Err(format!(
            "Invalid database extension '.{}'. Allowed: {}",
            e,
            ALLOWED_DB_EXTENSIONS.join(", ")
        )),
        None => Err("Database file must have .h5 or .hdf5 extension".to_string()),
    }
}

/// Validate image path has allowed extension.
#[cfg(feature = "python")]
fn validate_image_path(path: &str) -> Result<(), String> {
    let path = Path::new(path);

    // Check extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext {
        Some(ref e) if ALLOWED_IMAGE_EXTENSIONS.contains(&e.as_str()) => Ok(()),
        Some(e) => Err(format!(
            "Invalid image extension '.{}'. Allowed: {}",
            e,
            ALLOWED_IMAGE_EXTENSIONS.join(", ")
        )),
        None => Err("Image file must have a valid extension (png, jpg, etc.)".to_string()),
    }
}

/// Main stockpile scanner interface.
#[cfg(feature = "python")]
#[pyclass]
pub struct StockpileScanner {
    /// Internal scan pipeline.
    pipeline: ScanPipeline,
}

#[cfg(feature = "python")]
#[pymethods]
impl StockpileScanner {
    /// Create a new stockpile scanner.
    ///
    /// Args:
    ///     database_path: Path to the HDF5 template database.
    ///     data_path: Path to the OCR models directory (default: "data").
    ///
    /// Returns:
    ///     A new StockpileScanner instance.
    #[new]
    #[pyo3(signature = (database_path, data_path=None))]
    pub fn new(database_path: &str, data_path: Option<&str>) -> PyResult<Self> {
        let data_dir = data_path.unwrap_or("data");

        // Validate database path extension (security: prevent loading arbitrary files)
        validate_database_path(database_path).map_err(pyo3::exceptions::PyValueError::new_err)?;

        // Check file exists
        if !Path::new(database_path).exists() {
            // Don't expose full path in error for security
            return Err(pyo3::exceptions::PyFileNotFoundError::new_err(
                "Database file not found",
            ));
        }

        let config = ScanConfig::default();
        let pipeline = ScanPipeline::new(database_path, data_dir, config);

        Ok(Self { pipeline })
    }

    /// Scan a stockpile screenshot.
    ///
    /// Args:
    ///     image: NumPy array (H x W x 3, uint8, BGR format).
    ///     faction: Optional faction filter ("wardens", "colonials", or None for all).
    ///     config: Optional scan configuration.
    ///
    /// Returns:
    ///     Stockpile result with detected items and metadata.
    #[pyo3(signature = (image, faction=None, config=None))]
    pub fn scan(
        &mut self,
        image: PyReadonlyArray3<u8>,
        faction: Option<&str>,
        config: Option<ScanConfig>,
    ) -> PyResult<Stockpile> {
        // Update config if provided
        if let Some(cfg) = config {
            self.pipeline.set_config(cfg);
        }

        // Parse faction
        let faction_enum = faction.map(|f| ItemFaction::from_string(Some(f)));

        // Get image dimensions
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;

        // Get raw bytes
        let image_data = image.as_slice()?;

        // Run the pipeline
        self.pipeline
            .scan(image_data, width, height, faction_enum)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Scan a stockpile screenshot from file path.
    ///
    /// Args:
    ///     image_path: Path to the image file.
    ///     faction: Optional faction filter.
    ///     config: Optional scan configuration.
    ///
    /// Returns:
    ///     Stockpile result with detected items and metadata.
    #[pyo3(signature = (image_path, faction=None, config=None))]
    pub fn scan_file(
        &mut self,
        image_path: &str,
        faction: Option<&str>,
        config: Option<ScanConfig>,
    ) -> PyResult<Stockpile> {
        // Validate image path extension (security: prevent loading arbitrary files)
        validate_image_path(image_path).map_err(pyo3::exceptions::PyValueError::new_err)?;

        // Update config if provided
        if let Some(cfg) = config {
            self.pipeline.set_config(cfg);
        }

        // Load image from file (don't expose full path in error)
        let img = image::open(image_path)
            .map_err(|_| pyo3::exceptions::PyIOError::new_err("Failed to load image file"))?;

        let rgb = img.to_rgb8();
        let (width, height) = rgb.dimensions();
        let image_data = rgb.into_raw();

        // Parse faction
        let faction_enum = faction.map(|f| ItemFaction::from_string(Some(f)));

        // Run the pipeline
        self.pipeline
            .scan(&image_data, width as i32, height as i32, faction_enum)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Get the current configuration.
    pub fn get_config(&self) -> ScanConfig {
        self.pipeline.config().clone()
    }

    /// Set the configuration.
    pub fn set_config(&mut self, config: ScanConfig) {
        self.pipeline.set_config(config);
    }

    /// Get the database path.
    pub fn database_path(&self) -> &str {
        self.pipeline.database_path()
    }

    /// Get the data path (OCR models directory).
    pub fn data_path(&self) -> &str {
        self.pipeline.data_path()
    }

    /// Preload the database and OCR engines for fast subsequent scans.
    ///
    /// Call this at server startup to avoid cold start penalty on the first request.
    /// After preloading, all scans will be "warm" scans with consistent timing.
    ///
    /// Args:
    ///     resolution: Target resolution height (e.g., 1080, 1440, 2160).
    ///                 Use 2160 to preload 4K templates.
    ///
    /// Example:
    ///     scanner = StockpileScanner("templates.h5", "data")
    ///     scanner.preload(2160)  # Load 4K templates at startup
    ///     # Now all scans will be fast
    #[pyo3(signature = (resolution=2160))]
    pub fn preload(&mut self, resolution: i32) -> PyResult<()> {
        self.pipeline
            .preload(resolution)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    /// Check if the scanner has preloaded its resources.
    pub fn is_preloaded(&self) -> bool {
        self.pipeline.is_preloaded()
    }

    /// Eagerly build the masked OCR engines for the shard name and the
    /// per-script timestamp lines, so the first scan of each script doesn't pay
    /// the model-load cost. Optional: long-lived scanners benefit from calling
    /// it once at startup; single-shot CLI use can skip it (engines build
    /// lazily).
    pub fn warmup(&self) -> PyResult<()> {
        self.pipeline
            .warmup()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }

    fn __repr__(&self) -> String {
        format!(
            "StockpileScanner(database='{}', data='{}')",
            self.pipeline.database_path(),
            self.pipeline.data_path()
        )
    }
}

/// Compute perceptual hash for a BGR image (for debugging/verification).
///
/// This matches Python's compute_icon_phash() exactly.
///
/// Args:
///     image: NumPy array (H x W x 3, uint8, BGR format).
///
/// Returns:
///     64-bit perceptual hash as integer.
#[cfg(feature = "python")]
#[pyfunction]
fn compute_phash(image: PyReadonlyArray3<u8>) -> PyResult<u64> {
    let shape = image.shape();
    let height = shape[0];
    let width = shape[1];
    let image_data = image.as_slice()?;

    Ok(template::phash::compute_phash(image_data, width, height))
}

/// Python module definition.
///
/// Built as the native sub-module `fs_ocr._fs_ocr`; the `fs_ocr` Python package
/// (see `python/fs_ocr/__init__.py`) re-exports these symbols.
#[cfg(feature = "python")]
#[pymodule]
fn _fs_ocr(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Main scanner class
    m.add_class::<StockpileScanner>()?;

    // Configuration
    m.add_class::<ScanConfig>()?;

    // Enums
    m.add_class::<StockpileType>()?;
    m.add_class::<ItemFaction>()?;
    m.add_class::<ItemCategory>()?;

    // Models
    m.add_class::<Stockpile>()?;
    m.add_class::<StockpileItem>()?;
    m.add_class::<ItemCandidate>()?;
    m.add_class::<Timing>()?;

    // Debug/verification functions
    m.add_function(wrap_pyfunction!(compute_phash, m)?)?;

    // Module metadata
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("__author__", env!("CARGO_PKG_AUTHORS"))?;

    // OCR backend info. Text is always recognized by the embedded pure-Rust
    // ocrs model. Chinese *custom names* are additionally read via the system
    // `tesseract` CLI when it is installed (detected at runtime, not build time).
    m.add("OCR_BACKEND", "ocrs")?;

    Ok(())
}

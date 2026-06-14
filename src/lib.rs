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

pub mod config;
pub mod constants;
pub mod coordinator;
pub mod detector;
pub mod enums;
pub mod error;
pub mod image_utils;
pub mod models;
pub mod ocr;
pub mod template;
pub mod text_utils;

#[cfg(feature = "python")]
use config::ScanConfig;
#[cfg(feature = "python")]
use coordinator::ScanPipeline;
#[cfg(feature = "python")]
use detector::{BlackBoxDetector, GreyMaskDetector};
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

    /// Debug: Get detected quantity box positions (full image detection).
    ///
    /// Returns a list of (x, y) tuples for each detected quantity box.
    /// This uses full-image grey mask detection with morphology and adaptive threshold.
    #[pyo3(signature = (image,))]
    pub fn debug_detect_boxes(&self, image: PyReadonlyArray3<u8>) -> PyResult<Vec<(i32, i32)>> {
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;
        let image_data = image.as_slice()?;

        let detector = GreyMaskDetector::new(width, height);
        let regions = detector
            .detect(image_data, width, height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        Ok(regions.quantity_boxes)
    }

    /// Debug: Get detected quantity box positions using ROI-based pipeline.
    ///
    /// This mimics the actual scan() pipeline: black box ROI detection + grey mask on ROI.
    /// Returns a list of (x, y) tuples for each detected quantity box.
    #[pyo3(signature = (image,))]
    pub fn debug_detect_boxes_roi(&self, image: PyReadonlyArray3<u8>) -> PyResult<Vec<(i32, i32)>> {
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;
        let image_data = image.as_slice()?;

        // Step 1: Black box detection to find ROI
        let bb_detector = BlackBoxDetector::new(width, height);
        let bb_result = bb_detector
            .detect(image_data, width, height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        let roi = match bb_result {
            Some(r) => r.roi,
            None => {
                // Fall back to full detection
                let detector = GreyMaskDetector::new(width, height);
                let regions = detector
                    .detect(image_data, width, height)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;
                return Ok(regions.quantity_boxes);
            }
        };

        let (roi_x, roi_y, roi_w, roi_h) = roi;

        // Step 2: Fast "not black" detection on ROI
        let detector = GreyMaskDetector::new(width, height);
        let mut regions = detector
            .detect_roi_fast(image_data, width, height, roi_x, roi_y, roi_w, roi_h)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        // Step 3: Adjust coordinates to full image space
        for (x, y) in &mut regions.quantity_boxes {
            *x += roi_x;
            *y += roi_y;
        }

        Ok(regions.quantity_boxes)
    }

    /// Debug: Get all detected contours before filtering.
    ///
    /// Returns a list of (x, y, width, height) tuples for all contours.
    #[pyo3(signature = (image,))]
    pub fn debug_detect_all_contours(
        &self,
        image: PyReadonlyArray3<u8>,
    ) -> PyResult<Vec<(i32, i32, i32, i32)>> {
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;
        let image_data = image.as_slice()?;

        let detector = GreyMaskDetector::new(width, height);
        let contours = detector
            .debug_get_all_contours(image_data, width, height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        Ok(contours)
    }

    /// Debug: Detect black box region (stockpile ROI).
    ///
    /// Returns the ROI bounding box: (roi_x, roi_y, roi_w, roi_h)
    #[pyo3(signature = (image,))]
    pub fn debug_detect_black_boxes(
        &self,
        image: PyReadonlyArray3<u8>,
    ) -> PyResult<Option<(i32, i32, i32, i32)>> {
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;
        let image_data = image.as_slice()?;

        let detector = BlackBoxDetector::new(width, height);
        let result = detector
            .detect(image_data, width, height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        Ok(result.map(|r| r.roi))
    }

    /// Debug: Get detected regions including type_region, name_region, shard_region.
    ///
    /// Returns a dict with region info for debugging OCR.
    #[pyo3(signature = (image,))]
    pub fn debug_detect_regions(
        &mut self,
        image: PyReadonlyArray3<u8>,
    ) -> PyResult<pyo3::Py<pyo3::types::PyDict>> {
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;
        let image_data = image.as_slice()?;

        // Initialize to set up text extractors
        self.pipeline
            .ensure_initialized_public(height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        // Detect regions
        let (regions, _, _) = self
            .pipeline
            .detect_stockpile_regions_public(image_data, width, height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        // Extract OCR text from type region if available
        let type_text = if let Some((x, y, w, h)) = regions.type_region {
            self.pipeline
                .extract_text_from_region_public(image_data, width, height, x, y, w, h)
                .unwrap_or_else(|_| "(OCR error)".to_string())
        } else {
            "(no type region)".to_string()
        };

        // Build result dict
        Python::with_gil(|py| {
            let dict = pyo3::types::PyDict::new(py);
            dict.set_item("info_bar_height", regions.info_bar_height)?;
            dict.set_item("type_region", regions.type_region)?;
            dict.set_item("name_region", regions.name_region)?;
            dict.set_item("shard_region", regions.shard_region)?;
            dict.set_item("box_count", regions.quantity_boxes.len())?;
            dict.set_item("type_text_raw", type_text)?;
            Ok(dict.into())
        })
    }

    /// Debug: Recognize quantities using template matching (no OCR).
    ///
    /// Returns a list of recognized quantities for each detected box.
    /// Uses pure template matching against digit patterns.
    #[pyo3(signature = (image,))]
    pub fn debug_recognize_quantities_template(
        &self,
        image: PyReadonlyArray3<u8>,
    ) -> PyResult<Vec<i32>> {
        let shape = image.shape();
        let height = shape[0] as i32;
        let width = shape[1] as i32;
        let image_data = image.as_slice()?;

        // Convert RGB to grayscale
        let grayscale =
            crate::image_utils::rgb_to_grayscale(image_data, width as usize, height as usize);

        // Get quantity boxes
        let bb_detector = BlackBoxDetector::new(width, height);
        let bb_result = bb_detector
            .detect(image_data, width, height)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        let (roi_x, roi_y, roi_w, roi_h) = match bb_result {
            Some(r) => r.roi,
            None => return Ok(Vec::new()),
        };

        let detector = GreyMaskDetector::new(width, height);
        let mut regions = detector
            .detect_roi_fast(image_data, width, height, roi_x, roi_y, roi_w, roi_h)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        // Adjust coordinates
        for (x, y) in &mut regions.quantity_boxes {
            *x += roi_x;
            *y += roi_y;
        }

        let scale = height as f64 / 2160.0;
        let box_width = (84.0 * scale) as i32;
        let box_height = (64.0 * scale) as i32;

        // Recognize using template matching
        let quantities = ocr::digit_matcher::recognize_quantities_batch(
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

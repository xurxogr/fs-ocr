//! Error types for the fs-ocr library.

#[cfg(feature = "python")]
use pyo3::exceptions::PyException;
#[cfg(feature = "python")]
use pyo3::prelude::*;
use thiserror::Error;

/// Main error type for the fs-ocr library.
#[derive(Error, Debug)]
pub enum FsOcrError {
    /// Failed to load or parse the HDF5 database.
    #[error("Database error: {0}")]
    Database(String),

    /// Failed to open or read the HDF5 file.
    #[error("HDF5 error: {0}")]
    Hdf5(String),

    /// Image processing error.
    #[error("Image processing error: {0}")]
    Image(String),

    /// OpenCV operation failed.
    #[error("OpenCV error: {0}")]
    OpenCv(String),

    /// Tesseract OCR error.
    #[error("OCR error: {0}")]
    Ocr(String),

    /// Invalid configuration.
    #[error("Configuration error: {0}")]
    Config(String),

    /// No stockpile detected in the image.
    #[error("No stockpile detected in image")]
    NoStockpileDetected,

    /// Resolution not supported.
    #[error("Unsupported resolution: {0}")]
    UnsupportedResolution(i32),

    /// Template not found.
    #[error("Template not found: {0}")]
    TemplateNotFound(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Generic internal error.
    #[error("Internal error: {0}")]
    Internal(String),
}

#[cfg(feature = "python")]
impl From<FsOcrError> for PyErr {
    fn from(err: FsOcrError) -> PyErr {
        PyException::new_err(err.to_string())
    }
}

/// Result type alias for fs-ocr operations.
pub type Result<T> = std::result::Result<T, FsOcrError>;

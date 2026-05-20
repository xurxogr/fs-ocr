//! Pipeline timing metrics.

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

/// Per-stage timing for a scan (all values in milliseconds).
#[pyclass]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timing {
    /// Total detection stage.
    #[pyo3(get, set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection_ms: Option<f64>,

    /// Black box ROI detection (part of detection).
    #[pyo3(get, set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blackbox_ms: Option<f64>,

    /// Grey mask detection (part of detection).
    #[pyo3(get, set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub greymask_ms: Option<f64>,

    /// Quantity OCR / template matching.
    #[pyo3(get, set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_ms: Option<f64>,

    /// Icon template matching.
    #[pyo3(get, set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_ms: Option<f64>,

    /// Metadata extraction (type, name, shard, timestamp).
    #[pyo3(get, set)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_ms: Option<f64>,
}

#[pymethods]
impl Timing {
    #[new]
    pub fn new() -> Self {
        Self::default()
    }

    fn __repr__(&self) -> String {
        format!(
            "Timing(detection={:?}, blackbox={:?}, greymask={:?}, quantity={:?}, matching={:?}, metadata={:?})",
            self.detection_ms,
            self.blackbox_ms,
            self.greymask_ms,
            self.quantity_ms,
            self.matching_ms,
            self.metadata_ms,
        )
    }
}

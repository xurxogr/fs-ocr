//! Pipeline timing metrics.

#[cfg(feature = "python")]
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

/// Per-stage timing for a scan (all values in milliseconds).
#[cfg_attr(feature = "python", pyclass)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timing {
    /// Total detection stage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detection_ms: Option<f64>,

    /// Black box ROI detection (part of detection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blackbox_ms: Option<f64>,

    /// Grey mask detection (part of detection).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub greymask_ms: Option<f64>,

    /// Quantity OCR / template matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_ms: Option<f64>,

    /// Icon template matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matching_ms: Option<f64>,

    /// Metadata extraction (type, name, shard, timestamp).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_ms: Option<f64>,
}

#[cfg(feature = "python")]
#[pymethods]
impl Timing {
    #[new]
    fn py_new() -> Self {
        Self::default()
    }

    #[getter]
    fn detection_ms(&self) -> Option<f64> {
        self.detection_ms
    }
    #[setter]
    fn set_detection_ms(&mut self, value: Option<f64>) {
        self.detection_ms = value;
    }

    #[getter]
    fn blackbox_ms(&self) -> Option<f64> {
        self.blackbox_ms
    }
    #[setter]
    fn set_blackbox_ms(&mut self, value: Option<f64>) {
        self.blackbox_ms = value;
    }

    #[getter]
    fn greymask_ms(&self) -> Option<f64> {
        self.greymask_ms
    }
    #[setter]
    fn set_greymask_ms(&mut self, value: Option<f64>) {
        self.greymask_ms = value;
    }

    #[getter]
    fn quantity_ms(&self) -> Option<f64> {
        self.quantity_ms
    }
    #[setter]
    fn set_quantity_ms(&mut self, value: Option<f64>) {
        self.quantity_ms = value;
    }

    #[getter]
    fn matching_ms(&self) -> Option<f64> {
        self.matching_ms
    }
    #[setter]
    fn set_matching_ms(&mut self, value: Option<f64>) {
        self.matching_ms = value;
    }

    #[getter]
    fn metadata_ms(&self) -> Option<f64> {
        self.metadata_ms
    }
    #[setter]
    fn set_metadata_ms(&mut self, value: Option<f64>) {
        self.metadata_ms = value;
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

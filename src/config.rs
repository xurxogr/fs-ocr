//! Configuration types for the OCR scanner.

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use crate::constants::{MAX_NCC_CANDIDATES, NCC_TIEBREAKER_THRESHOLD, PHASH_THRESHOLD};

/// Configuration for the stockpile scanner.
#[pyclass]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    /// Maximum Hamming distance for pHash filtering.
    /// Lower value = fewer candidates, faster but may miss matches.
    /// Default: 15
    #[pyo3(get, set)]
    pub phash_threshold: u32,

    /// Maximum candidates to evaluate with NCC after pHash filtering.
    /// Default: 50
    #[pyo3(get, set)]
    pub max_ncc_candidates: usize,

    /// Confidence gap for returning alternative candidates.
    /// When > 0, returns all matches within this gap of the best match.
    /// Default: 0.0 (only return best match)
    #[pyo3(get, set)]
    pub confidence_gap: f64,

    /// NCC tiebreaker threshold.
    /// When top matches are within this threshold, use edge-based comparison.
    /// Set to 0.0 to disable tiebreaker.
    /// Default: 0.0015
    #[pyo3(get, set)]
    pub ncc_tiebreaker_threshold: f64,
}

#[pymethods]
impl ScanConfig {
    /// Create a new ScanConfig with default values.
    #[new]
    #[pyo3(signature = (
        phash_threshold=None,
        max_ncc_candidates=None,
        confidence_gap=0.0,
        ncc_tiebreaker_threshold=None
    ))]
    pub fn new(
        phash_threshold: Option<u32>,
        max_ncc_candidates: Option<usize>,
        confidence_gap: f64,
        ncc_tiebreaker_threshold: Option<f64>,
    ) -> Self {
        Self {
            phash_threshold: phash_threshold.unwrap_or(PHASH_THRESHOLD),
            max_ncc_candidates: max_ncc_candidates.unwrap_or(MAX_NCC_CANDIDATES),
            confidence_gap,
            ncc_tiebreaker_threshold: ncc_tiebreaker_threshold.unwrap_or(NCC_TIEBREAKER_THRESHOLD),
        }
    }

    /// Create a config from a JSON string.
    #[staticmethod]
    pub fn from_json(json: &str) -> PyResult<Self> {
        serde_json::from_str(json)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("Invalid JSON: {}", e)))
    }

    /// Serialize the config to JSON.
    pub fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(self).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("JSON serialization failed: {}", e))
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "ScanConfig(phash_threshold={}, max_ncc_candidates={}, confidence_gap={}, \
             ncc_tiebreaker_threshold={})",
            self.phash_threshold,
            self.max_ncc_candidates,
            self.confidence_gap,
            self.ncc_tiebreaker_threshold
        )
    }
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            phash_threshold: PHASH_THRESHOLD,
            max_ncc_candidates: MAX_NCC_CANDIDATES,
            confidence_gap: 0.0,
            ncc_tiebreaker_threshold: NCC_TIEBREAKER_THRESHOLD,
        }
    }
}

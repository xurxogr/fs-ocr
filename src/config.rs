//! Configuration types for the OCR scanner.

#[cfg(feature = "python")]
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use crate::constants::{
    MAX_NCC_CANDIDATES, NCC_ESCALATION_THRESHOLD, NCC_INITIAL_CANDIDATES, NCC_TIEBREAKER_THRESHOLD,
    PHASH_THRESHOLD,
};

/// Configuration for the stockpile scanner.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    /// Maximum Hamming distance for pHash filtering.
    /// Lower value = fewer candidates, faster but may miss matches.
    /// Default: 15
    pub phash_threshold: u32,

    /// Hard cap on candidates evaluated with NCC after pHash filtering.
    /// Upper bound of adaptive escalation. Default: 100
    pub max_ncc_candidates: usize,

    /// Initial NCC batch size for adaptive escalation.
    /// Matching starts by scoring this many top-pHash candidates and only
    /// expands when confidence stays low. Default: 25
    pub ncc_initial_candidates: usize,

    /// Confidence floor for adaptive escalation.
    /// If the best confidence after a batch is below this, the candidate count
    /// is doubled (up to `max_ncc_candidates`) and matching retries.
    /// Default: 0.85
    pub ncc_escalation_threshold: f64,

    /// Confidence gap for returning alternative candidates.
    /// When > 0, returns all matches within this gap of the best match.
    /// Default: 0.0 (only return best match)
    pub confidence_gap: f64,

    /// NCC tiebreaker threshold.
    /// When top matches are within this threshold, use edge-based comparison.
    /// Set to 0.0 to disable tiebreaker.
    /// Default: 0.0015
    pub ncc_tiebreaker_threshold: f64,
}

#[cfg(feature = "python")]
#[pymethods]
impl ScanConfig {
    /// Create a new ScanConfig with default values.
    #[new]
    #[pyo3(signature = (
        phash_threshold=None,
        max_ncc_candidates=None,
        confidence_gap=0.0,
        ncc_tiebreaker_threshold=None,
        ncc_initial_candidates=None,
        ncc_escalation_threshold=None
    ))]
    fn py_new(
        phash_threshold: Option<u32>,
        max_ncc_candidates: Option<usize>,
        confidence_gap: f64,
        ncc_tiebreaker_threshold: Option<f64>,
        ncc_initial_candidates: Option<usize>,
        ncc_escalation_threshold: Option<f64>,
    ) -> Self {
        Self {
            phash_threshold: phash_threshold.unwrap_or(PHASH_THRESHOLD),
            max_ncc_candidates: max_ncc_candidates.unwrap_or(MAX_NCC_CANDIDATES),
            confidence_gap,
            ncc_tiebreaker_threshold: ncc_tiebreaker_threshold.unwrap_or(NCC_TIEBREAKER_THRESHOLD),
            ncc_initial_candidates: ncc_initial_candidates.unwrap_or(NCC_INITIAL_CANDIDATES),
            ncc_escalation_threshold: ncc_escalation_threshold.unwrap_or(NCC_ESCALATION_THRESHOLD),
        }
    }

    #[getter]
    fn phash_threshold(&self) -> u32 {
        self.phash_threshold
    }
    #[setter]
    fn set_phash_threshold(&mut self, value: u32) {
        self.phash_threshold = value;
    }

    #[getter]
    fn max_ncc_candidates(&self) -> usize {
        self.max_ncc_candidates
    }
    #[setter]
    fn set_max_ncc_candidates(&mut self, value: usize) {
        self.max_ncc_candidates = value;
    }

    #[getter]
    fn ncc_initial_candidates(&self) -> usize {
        self.ncc_initial_candidates
    }
    #[setter]
    fn set_ncc_initial_candidates(&mut self, value: usize) {
        self.ncc_initial_candidates = value;
    }

    #[getter]
    fn ncc_escalation_threshold(&self) -> f64 {
        self.ncc_escalation_threshold
    }
    #[setter]
    fn set_ncc_escalation_threshold(&mut self, value: f64) {
        self.ncc_escalation_threshold = value;
    }

    #[getter]
    fn confidence_gap(&self) -> f64 {
        self.confidence_gap
    }
    #[setter]
    fn set_confidence_gap(&mut self, value: f64) {
        self.confidence_gap = value;
    }

    #[getter]
    fn ncc_tiebreaker_threshold(&self) -> f64 {
        self.ncc_tiebreaker_threshold
    }
    #[setter]
    fn set_ncc_tiebreaker_threshold(&mut self, value: f64) {
        self.ncc_tiebreaker_threshold = value;
    }

    /// Create a config from a JSON string.
    #[staticmethod]
    fn from_json(json: &str) -> PyResult<Self> {
        serde_json::from_str(json)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("Invalid JSON: {}", e)))
    }

    /// Serialize the config to JSON.
    fn to_json(&self) -> PyResult<String> {
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
            ncc_initial_candidates: NCC_INITIAL_CANDIDATES,
            ncc_escalation_threshold: NCC_ESCALATION_THRESHOLD,
        }
    }
}

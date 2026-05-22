//! Stockpile item model representing a single detected item.

#[cfg(feature = "python")]
use pyo3::prelude::*;
use serde::{Deserialize, Serialize, Serializer};

/// Serialize f64 rounded to 3 decimals (matches the fs reference output).
fn serialize_confidence<S: Serializer>(value: &f64, ser: S) -> Result<S::Ok, S::Error> {
    let rounded = (value * 1000.0).round() / 1000.0;
    ser.serialize_f64(rounded)
}

/// An alternative candidate match for an item.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemCandidate {
    /// Item code identifier.
    pub code: String,

    /// Match confidence (0.0 - 1.0).
    #[serde(serialize_with = "serialize_confidence")]
    pub confidence: f64,
}

impl ItemCandidate {
    /// Create a new item candidate.
    pub fn new(code: String, confidence: f64) -> Self {
        Self { code, confidence }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl ItemCandidate {
    #[new]
    fn py_new(code: String, confidence: f64) -> Self {
        Self::new(code, confidence)
    }

    #[getter]
    fn code(&self) -> String {
        self.code.clone()
    }

    #[getter]
    fn confidence(&self) -> f64 {
        self.confidence
    }

    fn __repr__(&self) -> String {
        format!(
            "ItemCandidate(code='{}', confidence={:.4})",
            self.code, self.confidence
        )
    }
}

/// A single item detected in a stockpile.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockpileItem {
    /// Item code identifier (e.g., "RifleC", "rmg_40").
    /// Set to "Unknown" if no match was found.
    pub code: String,

    /// Detected quantity. -1 indicates OCR failure.
    pub quantity: i32,

    /// Whether the item is in crated form.
    pub crated: bool,

    /// Match confidence (0.0 - 1.0).
    #[serde(serialize_with = "serialize_confidence")]
    pub confidence: f64,

    /// Alternative candidates within the confidence gap (if configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<ItemCandidate>>,
}

impl StockpileItem {
    /// Create a new stockpile item.
    pub fn new(
        code: String,
        quantity: i32,
        crated: bool,
        confidence: f64,
        candidates: Option<Vec<ItemCandidate>>,
    ) -> Self {
        Self {
            code,
            quantity,
            crated,
            confidence,
            candidates,
        }
    }

    /// Create an unknown item (failed to match).
    pub fn unknown(quantity: i32, crated: bool) -> Self {
        Self {
            code: "Unknown".to_string(),
            quantity,
            crated,
            confidence: 0.0,
            candidates: None,
        }
    }

    /// Check if this item was successfully matched.
    pub fn is_matched(&self) -> bool {
        self.code != "Unknown"
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl StockpileItem {
    #[new]
    #[pyo3(signature = (code, quantity, crated=false, confidence=0.0, candidates=None))]
    fn py_new(
        code: String,
        quantity: i32,
        crated: bool,
        confidence: f64,
        candidates: Option<Vec<ItemCandidate>>,
    ) -> Self {
        Self::new(code, quantity, crated, confidence, candidates)
    }

    /// Create an unknown item (failed to match).
    #[staticmethod]
    #[pyo3(name = "unknown")]
    fn py_unknown(quantity: i32, crated: bool) -> Self {
        Self::unknown(quantity, crated)
    }

    #[getter]
    fn code(&self) -> String {
        self.code.clone()
    }

    #[getter]
    fn quantity(&self) -> i32 {
        self.quantity
    }

    #[getter]
    fn crated(&self) -> bool {
        self.crated
    }

    #[getter]
    fn confidence(&self) -> f64 {
        self.confidence
    }

    #[getter]
    fn candidates(&self) -> Option<Vec<ItemCandidate>> {
        self.candidates.clone()
    }

    #[pyo3(name = "is_matched")]
    fn py_is_matched(&self) -> bool {
        self.is_matched()
    }

    /// Serialize to JSON string.
    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(self)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("JSON error: {}", e)))
    }

    fn __repr__(&self) -> String {
        format!(
            "StockpileItem(code='{}', quantity={}, crated={}, confidence={:.4})",
            self.code, self.quantity, self.crated, self.confidence
        )
    }
}

impl Default for StockpileItem {
    fn default() -> Self {
        Self::unknown(-1, false)
    }
}

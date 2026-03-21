//! Stockpile model representing the complete scan result.

use chrono::{DateTime, Utc};
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use super::StockpileItem;
use crate::enums::StockpileType;

/// Complete stockpile scan result.
#[pyclass]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stockpile {
    /// Custom stockpile name (if applicable).
    #[pyo3(get, set)]
    pub name: Option<String>,

    /// Detected stockpile type.
    #[pyo3(get, set)]
    pub stockpile_type: StockpileType,

    /// List of detected items with quantities.
    #[pyo3(get)]
    pub items: Vec<StockpileItem>,

    /// Scan timestamp (ISO 8601 format).
    #[pyo3(get)]
    pub timestamp: String,

    /// Game shard name (e.g., "ABLE", "BAKER").
    #[pyo3(get, set)]
    pub shard: Option<String>,

    /// In-game timestamp (e.g., "Day 1293, 1906 Hours").
    #[pyo3(get, set)]
    pub ingame_timestamp: Option<String>,

    /// Screenshot resolution (e.g., "1920x1080").
    #[pyo3(get)]
    pub resolution: String,

    /// Processing errors or warnings.
    #[pyo3(get)]
    pub errors: Vec<String>,

    /// Timing: detection stage (ms).
    #[pyo3(get)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_detection_ms: Option<f64>,

    /// Timing: black box detection (ms) - part of detection.
    #[pyo3(get)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_blackbox_ms: Option<f64>,

    /// Timing: grey mask detection (ms) - part of detection.
    #[pyo3(get)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_greymask_ms: Option<f64>,

    /// Timing: quantity extraction (ms).
    #[pyo3(get)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_quantity_ms: Option<f64>,

    /// Timing: icon matching (ms).
    #[pyo3(get)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_matching_ms: Option<f64>,

    /// Timing: metadata extraction (ms).
    #[pyo3(get)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_metadata_ms: Option<f64>,
}

#[pymethods]
impl Stockpile {
    /// Create a new stockpile result.
    #[new]
    #[pyo3(signature = (resolution, stockpile_type=StockpileType::Undefined))]
    pub fn new(resolution: String, stockpile_type: StockpileType) -> Self {
        Self {
            name: None,
            stockpile_type,
            items: Vec::new(),
            timestamp: Utc::now().to_rfc3339(),
            shard: None,
            ingame_timestamp: None,
            resolution,
            errors: Vec::new(),
            timing_detection_ms: None,
            timing_blackbox_ms: None,
            timing_greymask_ms: None,
            timing_quantity_ms: None,
            timing_matching_ms: None,
            timing_metadata_ms: None,
        }
    }

    /// Get the total number of items.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Get the number of successfully matched items.
    pub fn matched_count(&self) -> usize {
        self.items.iter().filter(|i| i.is_matched()).count()
    }

    /// Get the number of crated items.
    pub fn crated_count(&self) -> usize {
        self.items.iter().filter(|i| i.crated).count()
    }

    /// Check if scanning completed without errors.
    pub fn is_successful(&self) -> bool {
        self.errors.is_empty()
    }

    /// Serialize to JSON string.
    pub fn to_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("JSON error: {}", e)))
    }

    /// Serialize to compact JSON string.
    pub fn to_json_compact(&self) -> PyResult<String> {
        serde_json::to_string(self)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("JSON error: {}", e)))
    }

    fn __repr__(&self) -> String {
        format!(
            "Stockpile(name={:?}, type={}, items={}, resolution='{}')",
            self.name,
            self.stockpile_type.display_name(),
            self.items.len(),
            self.resolution
        )
    }

    fn __len__(&self) -> usize {
        self.items.len()
    }
}

impl Stockpile {
    /// Add an item to the stockpile.
    pub fn add_item(&mut self, item: StockpileItem) {
        self.items.push(item);
    }

    /// Add an error message.
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
    }

    /// Set the timestamp from a DateTime.
    pub fn set_timestamp(&mut self, dt: DateTime<Utc>) {
        self.timestamp = dt.to_rfc3339();
    }
}

impl Default for Stockpile {
    fn default() -> Self {
        Self::new("0x0".to_string(), StockpileType::Undefined)
    }
}

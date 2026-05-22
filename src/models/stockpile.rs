//! Stockpile model representing the complete scan result.

use chrono::{DateTime, Utc};
#[cfg(feature = "python")]
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

use super::{StockpileItem, Timing};
use crate::enums::StockpileType;

/// Complete stockpile scan result.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stockpile {
    /// Custom stockpile name (if applicable).
    pub name: Option<String>,

    /// Detected stockpile type.
    #[serde(rename = "type")]
    pub stockpile_type: StockpileType,

    /// Whether this stockpile is reserved (has a custom name other than "Public").
    #[serde(rename = "is_reserve")]
    pub is_reserved: bool,

    /// List of detected items with quantities.
    pub items: Vec<StockpileItem>,

    /// Scan timestamp (ISO 8601 format).
    pub timestamp: String,

    /// Game shard name (e.g., "ABLE", "BAKER").
    pub shard: Option<String>,

    /// In-game timestamp (e.g., "Day 1293, 1906 Hours").
    pub ingame_timestamp: Option<String>,

    /// Screenshot resolution (e.g., "1920x1080").
    pub resolution: String,

    /// Processing errors or warnings.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,

    /// Per-stage pipeline timing (None when not collected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
}

impl Stockpile {
    /// Create a new stockpile result.
    pub fn new(resolution: String, stockpile_type: StockpileType) -> Self {
        Self {
            name: None,
            is_reserved: false,
            stockpile_type,
            items: Vec::new(),
            timestamp: Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            shard: None,
            ingame_timestamp: None,
            resolution,
            errors: Vec::new(),
            timing: None,
        }
    }

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
        self.timestamp = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl Stockpile {
    #[new]
    #[pyo3(signature = (resolution, stockpile_type=StockpileType::Undefined))]
    fn py_new(resolution: String, stockpile_type: StockpileType) -> Self {
        Self::new(resolution, stockpile_type)
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.name.clone()
    }
    #[setter]
    fn set_name(&mut self, value: Option<String>) {
        self.name = value;
    }

    #[getter]
    #[pyo3(name = "type")]
    fn get_type(&self) -> StockpileType {
        self.stockpile_type
    }
    #[setter]
    #[pyo3(name = "type")]
    fn set_type(&mut self, value: StockpileType) {
        self.stockpile_type = value;
    }

    #[getter(is_reserve)]
    fn get_is_reserve(&self) -> bool {
        self.is_reserved
    }
    #[setter(is_reserve)]
    fn set_is_reserve(&mut self, value: bool) {
        self.is_reserved = value;
    }

    #[getter]
    fn items(&self) -> Vec<StockpileItem> {
        self.items.clone()
    }

    #[getter]
    fn timestamp(&self) -> String {
        self.timestamp.clone()
    }

    #[getter]
    fn shard(&self) -> Option<String> {
        self.shard.clone()
    }
    #[setter]
    fn set_shard(&mut self, value: Option<String>) {
        self.shard = value;
    }

    #[getter]
    fn ingame_timestamp(&self) -> Option<String> {
        self.ingame_timestamp.clone()
    }
    #[setter]
    fn set_ingame_timestamp(&mut self, value: Option<String>) {
        self.ingame_timestamp = value;
    }

    #[getter]
    fn resolution(&self) -> String {
        self.resolution.clone()
    }

    #[getter]
    fn errors(&self) -> Vec<String> {
        self.errors.clone()
    }

    #[getter]
    fn timing(&self) -> Option<Timing> {
        self.timing.clone()
    }
    #[setter]
    fn set_timing(&mut self, value: Option<Timing>) {
        self.timing = value;
    }

    /// Get the total number of items.
    fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Get the number of successfully matched items.
    fn matched_count(&self) -> usize {
        self.items.iter().filter(|i| i.is_matched()).count()
    }

    /// Get the number of crated items.
    fn crated_count(&self) -> usize {
        self.items.iter().filter(|i| i.crated).count()
    }

    /// Check if scanning completed without errors.
    fn is_successful(&self) -> bool {
        self.errors.is_empty()
    }

    /// Serialize to JSON string.
    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("JSON error: {}", e)))
    }

    /// Serialize to compact JSON string.
    fn to_json_compact(&self) -> PyResult<String> {
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

impl Default for Stockpile {
    fn default() -> Self {
        Self::new("0x0".to_string(), StockpileType::Undefined)
    }
}

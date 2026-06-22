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

/// A diagnostic candidate produced by `scan_debug`.
///
/// Unlike [`ItemCandidate`] (the narrow, gap-restricted production
/// alternatives), this carries the full metadata the debug image viewer needs.
/// It represents one template that passed the icon's pHash threshold — any
/// code/category/mod/faction, matching the icon's crated state — NCC-scored.
#[cfg_attr(feature = "python", pyclass)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugCandidate {
    /// Item code identifier.
    pub code: String,

    /// NCC match confidence (TM_CCOEFF_NORMED), 0.0 - 1.0.
    #[serde(serialize_with = "serialize_confidence")]
    pub confidence: f64,

    /// Mod name (e.g. "vanilla", "airborne").
    #[serde(rename = "mod")]
    pub mod_name: String,

    /// Item category ("item", "vehicle", "shippable", "invalid").
    pub category: String,

    /// Whether the template is a crated item.
    pub crated: bool,

    /// Item faction ("neutral", "Colonials", "Wardens").
    pub faction: String,

    /// Hamming distance between the icon and template pHash (lower = closer).
    pub phash_distance: u32,
}

impl DebugCandidate {
    /// Create a new debug candidate.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        code: String,
        confidence: f64,
        mod_name: String,
        category: String,
        crated: bool,
        faction: String,
        phash_distance: u32,
    ) -> Self {
        Self {
            code,
            confidence,
            mod_name,
            category,
            crated,
            faction,
            phash_distance,
        }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl DebugCandidate {
    #[getter]
    fn code(&self) -> String {
        self.code.clone()
    }

    #[getter]
    fn confidence(&self) -> f64 {
        self.confidence
    }

    #[getter]
    #[pyo3(name = "mod")]
    fn mod_name(&self) -> String {
        self.mod_name.clone()
    }

    #[getter]
    fn category(&self) -> String {
        self.category.clone()
    }

    #[getter]
    fn crated(&self) -> bool {
        self.crated
    }

    #[getter]
    fn faction(&self) -> String {
        self.faction.clone()
    }

    #[getter]
    fn phash_distance(&self) -> u32 {
        self.phash_distance
    }

    fn __repr__(&self) -> String {
        format!(
            "DebugCandidate(code='{}', confidence={:.4}, mod='{}', category='{}', \
             crated={}, faction='{}', phash_distance={})",
            self.code,
            self.confidence,
            self.mod_name,
            self.category,
            self.crated,
            self.faction,
            self.phash_distance
        )
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

    /// X coordinate (pixels) of the icon region's top-left corner in the
    /// source screenshot.
    #[serde(default)]
    pub x: i32,

    /// Y coordinate (pixels) of the icon region's top-left corner in the
    /// source screenshot.
    #[serde(default)]
    pub y: i32,

    /// Alternative candidates within the confidence gap (if configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<ItemCandidate>>,

    /// Broad diagnostic candidate set, populated only by `scan_debug`.
    /// `None` (and omitted from JSON) for the normal `scan` path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_candidates: Option<Vec<DebugCandidate>>,
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
            x: 0,
            y: 0,
            candidates,
            debug_candidates: None,
        }
    }

    /// Create an unknown item (failed to match).
    pub fn unknown(quantity: i32, crated: bool) -> Self {
        Self {
            code: "Unknown".to_string(),
            quantity,
            crated,
            confidence: 0.0,
            x: 0,
            y: 0,
            candidates: None,
            debug_candidates: None,
        }
    }

    /// Return a copy of this item carrying the given debug candidate set.
    pub fn with_debug_candidates(self, debug_candidates: Vec<DebugCandidate>) -> Self {
        Self {
            debug_candidates: Some(debug_candidates),
            ..self
        }
    }

    /// Return a copy of this item positioned at the given icon coordinates.
    pub fn with_position(self, x: i32, y: i32) -> Self {
        Self { x, y, ..self }
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
    fn x(&self) -> i32 {
        self.x
    }

    #[getter]
    fn y(&self) -> i32 {
        self.y
    }

    #[getter]
    fn candidates(&self) -> Option<Vec<ItemCandidate>> {
        self.candidates.clone()
    }

    #[getter]
    fn debug_candidates(&self) -> Option<Vec<DebugCandidate>> {
        self.debug_candidates.clone()
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
            "StockpileItem(code='{}', quantity={}, crated={}, confidence={:.4}, x={}, y={})",
            self.code, self.quantity, self.crated, self.confidence, self.x, self.y
        )
    }
}

impl Default for StockpileItem {
    fn default() -> Self {
        Self::unknown(-1, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_candidate_json_unchanged_by_debug_fields() {
        // Production ItemCandidate must serialize to exactly code + confidence.
        let c = ItemCandidate::new("RifleC".to_string(), 0.987654);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"code":"RifleC","confidence":0.988}"#);
    }

    #[test]
    fn production_item_omits_debug_candidates() {
        // A normal (scan-path) item leaves debug_candidates None and must not
        // emit the key — keeps production output byte-for-byte unchanged.
        let item = StockpileItem::new("RifleC".to_string(), 5, false, 0.95, None);
        let json = serde_json::to_string(&item).unwrap();
        assert!(!json.contains("debug_candidates"));
    }

    #[test]
    fn debug_candidate_serializes_mod_key_and_metadata() {
        let dc = DebugCandidate::new(
            "RifleC".to_string(),
            0.912345,
            "vanilla".to_string(),
            "item".to_string(),
            false,
            "Colonials".to_string(),
            7,
        );
        let json = serde_json::to_string(&dc).unwrap();
        assert!(json.contains(r#""mod":"vanilla""#));
        assert!(json.contains(r#""category":"item""#));
        assert!(json.contains(r#""faction":"Colonials""#));
        assert!(json.contains(r#""phash_distance":7"#));
        assert!(json.contains(r#""confidence":0.912"#));
    }

    #[test]
    fn debug_item_round_trips_and_carries_candidates() {
        let dc = DebugCandidate::new(
            "RifleC".to_string(),
            1.0,
            "vanilla".to_string(),
            "item".to_string(),
            false,
            "neutral".to_string(),
            0,
        );
        let item = StockpileItem::new("RifleC".to_string(), 5, false, 1.0, None)
            .with_debug_candidates(vec![dc]);
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("debug_candidates"));

        let parsed: StockpileItem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.debug_candidates.unwrap().len(), 1);
    }

    #[test]
    fn item_json_without_debug_field_deserializes() {
        // Back-compat: older JSON without the key parses to None.
        let json = r#"{"code":"RifleC","quantity":5,"crated":false,"confidence":0.95,"x":0,"y":0}"#;
        let parsed: StockpileItem = serde_json::from_str(json).unwrap();
        assert!(parsed.debug_candidates.is_none());
    }
}

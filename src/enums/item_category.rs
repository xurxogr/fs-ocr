//! Item category enumeration.

#[cfg(feature = "python")]
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Item category classification.
#[cfg_attr(feature = "python", pyclass(eq, eq_int))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ItemCategory {
    /// Regular items (supplies, ammo, etc.).
    Item = 0,
    /// Vehicles (tanks, trucks, etc.).
    Vehicle = 1,
    /// Shippable/crated items.
    Shippable = 2,
    /// Invalid/unknown category.
    #[default]
    Invalid = 3,
}

impl ItemCategory {
    /// Get the display value for this category.
    pub fn value(&self) -> &'static str {
        match self {
            ItemCategory::Item => "item",
            ItemCategory::Vehicle => "vehicle",
            ItemCategory::Shippable => "shippable",
            ItemCategory::Invalid => "invalid",
        }
    }

    /// Parse from a string value.
    pub fn from_string(value: &str) -> Self {
        let normalized = value.trim().to_lowercase();
        match normalized.as_str() {
            "item" => ItemCategory::Item,
            "vehicle" => ItemCategory::Vehicle,
            "shippable" => ItemCategory::Shippable,
            _ => ItemCategory::Invalid,
        }
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl ItemCategory {
    #[pyo3(name = "value")]
    fn py_value(&self) -> &'static str {
        self.value()
    }

    #[staticmethod]
    #[pyo3(name = "from_string")]
    fn py_from_string(value: &str) -> Self {
        Self::from_string(value)
    }

    fn __repr__(&self) -> String {
        format!("ItemCategory.{}", self.value())
    }

    fn __str__(&self) -> String {
        self.value().to_string()
    }
}

impl fmt::Display for ItemCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

impl From<u8> for ItemCategory {
    fn from(value: u8) -> Self {
        match value {
            0 => ItemCategory::Item,
            1 => ItemCategory::Vehicle,
            2 => ItemCategory::Shippable,
            _ => ItemCategory::Invalid,
        }
    }
}

impl From<ItemCategory> for u8 {
    fn from(category: ItemCategory) -> Self {
        category as u8
    }
}

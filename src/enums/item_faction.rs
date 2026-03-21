//! Item faction enumeration.

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Faction variants for items.
#[pyclass(eq, eq_int)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ItemFaction {
    /// Neutral items available to both factions.
    #[default]
    Neutral = 0,
    /// Colonial faction items.
    Colonials = 1,
    /// Warden faction items.
    Wardens = 2,
}

#[pymethods]
impl ItemFaction {
    /// Convert a string to a Faction, never returns None.
    ///
    /// Args:
    ///     value: The string to convert. Can be a faction name, abbreviation, or None.
    ///
    /// Returns:
    ///     The corresponding Faction, defaults to NEUTRAL for invalid/empty input.
    #[staticmethod]
    #[pyo3(signature = (value=None))]
    pub fn from_string(value: Option<&str>) -> Self {
        match value {
            None => ItemFaction::Neutral,
            Some(s) => {
                let normalized = s.trim().to_lowercase();
                match normalized.as_str() {
                    "efactionid::colonials" | "colonials" | "c" => ItemFaction::Colonials,
                    "efactionid::wardens" | "wardens" | "w" => ItemFaction::Wardens,
                    _ => ItemFaction::Neutral,
                }
            }
        }
    }

    /// Get the display value for this faction.
    pub fn value(&self) -> &'static str {
        match self {
            ItemFaction::Neutral => "neutral",
            ItemFaction::Colonials => "Colonials",
            ItemFaction::Wardens => "Wardens",
        }
    }

    fn __repr__(&self) -> String {
        format!("ItemFaction.{}", self.value())
    }

    fn __str__(&self) -> String {
        self.value().to_string()
    }
}

impl fmt::Display for ItemFaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

impl From<u8> for ItemFaction {
    fn from(value: u8) -> Self {
        match value {
            0 => ItemFaction::Neutral,
            1 => ItemFaction::Colonials,
            2 => ItemFaction::Wardens,
            _ => ItemFaction::Neutral,
        }
    }
}

impl From<ItemFaction> for u8 {
    fn from(faction: ItemFaction) -> Self {
        faction as u8
    }
}

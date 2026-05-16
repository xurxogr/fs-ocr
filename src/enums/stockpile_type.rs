//! Stockpile type enumeration.

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Stockpile type indicating the kind of storage structure.
#[pyclass(eq, eq_int)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum StockpileType {
    // Bases (order from the game)
    /// Basic encampment structure.
    Encampment = 0,
    /// Keep structure.
    Keep = 1,
    /// Safe house structure.
    SafeHouse = 2,
    /// Relic base structure.
    RelicBase = 3,
    /// Bunker base (any level).
    BunkerBase = 4,
    /// Border base structure.
    BorderBase = 5,
    /// Town base (any level).
    TownBase = 6,
    /// Underground fortress.
    UndergroundFortress = 7,
    /// BMS Longhook structure.
    BmsLonghook = 8,

    // Structures (player-built, support custom names)
    /// Storage depot (supports custom name).
    StorageDepot = 9,
    /// Seaport (supports custom name).
    Seaport = 10,
    /// Aircraft depot (supports custom name).
    AircraftDepot = 11,

    /// Unknown or undetected type.
    #[default]
    Undefined = 12,
}

#[pymethods]
impl StockpileType {
    /// Check if this stockpile type supports custom player-given names.
    ///
    /// Only player-built structures (Storage Depot, Seaport, Aircraft Depot)
    /// can have custom names. Base types use their type as the display name.
    pub fn has_custom_name(&self) -> bool {
        matches!(
            self,
            StockpileType::StorageDepot | StockpileType::Seaport | StockpileType::AircraftDepot
        )
    }

    /// Get the display name for this stockpile type.
    pub fn display_name(&self) -> &'static str {
        match self {
            StockpileType::Encampment => "Encampment",
            StockpileType::Keep => "Keep",
            StockpileType::SafeHouse => "Safe House",
            StockpileType::RelicBase => "Relic Base",
            StockpileType::BunkerBase => "Bunker Base",
            StockpileType::BorderBase => "Border Base",
            StockpileType::TownBase => "Town Base",
            StockpileType::UndergroundFortress => "Underground Fortress",
            StockpileType::BmsLonghook => "BMS - Longhook",
            StockpileType::StorageDepot => "Storage Depot",
            StockpileType::Seaport => "Seaport",
            StockpileType::AircraftDepot => "Aircraft Depot",
            StockpileType::Undefined => "Undefined",
        }
    }

    /// Parse from a string value (supports multiple languages).
    #[staticmethod]
    pub fn from_string(value: &str) -> Self {
        Self::classify_from_text(value)
    }

    fn __repr__(&self) -> String {
        format!("StockpileType.{}", self.display_name())
    }

    fn __str__(&self) -> String {
        self.display_name().to_string()
    }
}

// Non-PyO3 methods (internal helpers)
impl StockpileType {
    /// Classify stockpile type from OCR text (supports multiple languages).
    pub fn classify_from_text(value: &str) -> Self {
        // Clean OCR artifacts
        let cleaned = value
            .trim()
            .trim_matches(|c| "'\"´`''«»|".contains(c))
            .trim();

        if cleaned.is_empty() {
            return StockpileType::Undefined;
        }

        // Try exact match first (case-insensitive)
        if let Some(t) = Self::exact_match_ignore_case(cleaned) {
            return t;
        }

        // Try fuzzy match for common OCR errors
        Self::fuzzy_match(cleaned)
    }

    /// Case-insensitive match against all known translations.
    fn exact_match_ignore_case(text: &str) -> Option<Self> {
        let text_lower = text.to_lowercase();

        // Helper to check if any item matches case-insensitively
        let matches = |list: &[&str]| list.iter().any(|s| s.to_lowercase() == text_lower);

        if matches(Self::ENCAMPMENT) {
            return Some(StockpileType::Encampment);
        }
        if matches(Self::KEEP) {
            return Some(StockpileType::Keep);
        }
        if matches(Self::SAFE_HOUSE) {
            return Some(StockpileType::SafeHouse);
        }
        if matches(Self::RELIC_BASE) {
            return Some(StockpileType::RelicBase);
        }
        if matches(Self::BUNKER_BASE) {
            return Some(StockpileType::BunkerBase);
        }
        if matches(Self::BORDER_BASE) {
            return Some(StockpileType::BorderBase);
        }
        if matches(Self::TOWN_BASE) {
            return Some(StockpileType::TownBase);
        }
        if matches(Self::UNDERGROUND_FORTRESS) {
            return Some(StockpileType::UndergroundFortress);
        }
        if matches(Self::BMS_LONGHOOK) {
            return Some(StockpileType::BmsLonghook);
        }
        if matches(Self::STORAGE_DEPOT) {
            return Some(StockpileType::StorageDepot);
        }
        if matches(Self::SEAPORT) {
            return Some(StockpileType::Seaport);
        }
        if matches(Self::AIRCRAFT_DEPOT) {
            return Some(StockpileType::AircraftDepot);
        }

        None
    }

    // Type name constants for reuse
    const ENCAMPMENT: &'static [&'static str] = &[
        "Encampment", "Feldlager", "Campement", "Acampamento", "Лагерь", "营地",
    ];
    const KEEP: &'static [&'static str] = &[
        "Keep", "Wehrturm", "Place Forte", "Torreão", "Крепость", "要塞",
    ];
    const SAFE_HOUSE: &'static [&'static str] = &[
        "Safe House", "Unterschlupf", "Planque", "Casa Fortificada", "Убежище", "安全屋",
    ];
    const RELIC_BASE: &'static [&'static str] = &[
        "Relic Base", "Reliktbasis", "Base Relique", "Base Relíquia", "Реликтовая База", "遗迹基地",
    ];
    const BUNKER_BASE: &'static [&'static str] = &[
        "Bunker Base", "Bunkerbasis", "Base Bunker", "Centro do Bunker", "Base de Bunker",
        "Centro do bunker", "Бункерная база", "Бункерная База", "地堡基地",
    ];
    const BORDER_BASE: &'static [&'static str] = &[
        "Border Base", "Grenzbasis", "Base Frontalière", "Base Fronteiriça", "Пограничная База", "边境基地",
    ];
    const TOWN_BASE: &'static [&'static str] = &[
        "Town Base", "Stadtkernbasis", "Quartier Général", "Base da Cidade", "Ратуша", "城镇基地",
    ];
    const UNDERGROUND_FORTRESS: &'static [&'static str] = &[
        "Underground Fortress", "Untergrundfestung", "Forteresse Souterraine", "Bunker Subterrâneo",
        "Подземная Крепость", "地下要塞",
    ];
    const BMS_LONGHOOK: &'static [&'static str] = &["BMS - Longhook"];
    const STORAGE_DEPOT: &'static [&'static str] = &[
        "Storage Depot", "Lagerdepot", "Dépôt", "Depósito", "Складское помещение", "仓库",
    ];
    const SEAPORT: &'static [&'static str] = &[
        "Seaport", "Seehafen", "Port", "Porto", "Морской порт", "海港",
    ];
    const AIRCRAFT_DEPOT: &'static [&'static str] = &["Aircraft Depot"];

    /// Exact match against all known translations (case-sensitive).
    fn exact_match(text: &str) -> Option<Self> {
        if Self::ENCAMPMENT.contains(&text) {
            return Some(StockpileType::Encampment);
        }
        if Self::KEEP.contains(&text) {
            return Some(StockpileType::Keep);
        }
        if Self::SAFE_HOUSE.contains(&text) {
            return Some(StockpileType::SafeHouse);
        }
        if Self::RELIC_BASE.contains(&text) {
            return Some(StockpileType::RelicBase);
        }
        if Self::BUNKER_BASE.contains(&text) {
            return Some(StockpileType::BunkerBase);
        }
        if Self::BORDER_BASE.contains(&text) {
            return Some(StockpileType::BorderBase);
        }
        if Self::TOWN_BASE.contains(&text) {
            return Some(StockpileType::TownBase);
        }
        if Self::UNDERGROUND_FORTRESS.contains(&text) {
            return Some(StockpileType::UndergroundFortress);
        }
        if Self::BMS_LONGHOOK.contains(&text) {
            return Some(StockpileType::BmsLonghook);
        }
        if Self::STORAGE_DEPOT.contains(&text) {
            return Some(StockpileType::StorageDepot);
        }
        if Self::SEAPORT.contains(&text) {
            return Some(StockpileType::Seaport);
        }
        if Self::AIRCRAFT_DEPOT.contains(&text) {
            return Some(StockpileType::AircraftDepot);
        }

        None
    }

    /// Fuzzy match for common OCR errors.
    fn fuzzy_match(text: &str) -> Self {
        // Try common OCR error substitutions
        let variations = [
            text.to_string(),
            text.replace('l', "I"),
            text.replace('I', "l"),
            text.replace('0', "O"),
            text.replace('O', "0"),
            text.replace('1', "I"),
            text.replace('5', "S"),
            text.replace('8', "B"),
        ];

        for variation in &variations {
            if let Some(t) = Self::exact_match(variation) {
                return t;
            }
        }

        StockpileType::Undefined
    }
}

impl fmt::Display for StockpileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

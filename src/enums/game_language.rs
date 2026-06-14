//! In-game client language, inferred from the stockpile-type template that
//! matches the type label.
//!
//! Finer-grained than the recognizer's script routing: a type-template hit
//! distinguishes every supported locale (e.g. German `Seehafen` vs French
//! `Port`), not just Latin/Cyrillic/CJK. Downstream OCR routing collapses these
//! back to a script as needed.

/// Supported in-game client languages, in the column order used by the
/// stockpile-type translation tables and the template asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GameLanguage {
    /// English (Latin).
    #[default]
    English = 0,
    /// German (Latin).
    German = 1,
    /// French (Latin).
    French = 2,
    /// Portuguese (Latin).
    Portuguese = 3,
    /// Russian (Cyrillic).
    Russian = 4,
    /// Chinese (Han).
    Chinese = 5,
}

impl GameLanguage {
    /// Map a stored discriminant (template `lang_id`) back to the enum.
    pub fn from_discriminant(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::English),
            1 => Some(Self::German),
            2 => Some(Self::French),
            3 => Some(Self::Portuguese),
            4 => Some(Self::Russian),
            5 => Some(Self::Chinese),
            _ => None,
        }
    }

    /// Whether this language is written in the Latin script (English routing).
    pub fn is_latin(self) -> bool {
        matches!(
            self,
            Self::English | Self::German | Self::French | Self::Portuguese
        )
    }
}

//! Stockpile-type label template matching.
//!
//! The stockpile type is a closed set of game-rendered strings — one per
//! language. This module owns the embedded type-label asset and maps a generic
//! [`label_match`] hit onto a [`StockpileType`] and the client's
//! [`GameLanguage`]. A match needs no OCR recognizer charset, so it covers
//! Chinese/Russian even though those are dropped from the recognizer.

use std::sync::OnceLock;

use crate::enums::{GameLanguage, StockpileType};
use crate::template::label_match::{self, LabelTemplate};

/// Embedded type-label asset; see `training/generate_type_templates.py` for the
/// format. Baked in at compile time, so there is no runtime file lookup.
pub(crate) static ASSET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/type_templates.bin"
));

/// Minimum NCC for a template to count as the type label. Below this the best
/// candidate is rejected and the caller falls back to OCR classification.
const MIN_NCC: f32 = 0.55;

/// A successful type-label match.
#[derive(Debug, Clone, Copy)]
pub struct TypeMatch {
    pub stype: StockpileType,
    pub lang: GameLanguage,
    pub score: f32,
}

static TEMPLATES: OnceLock<Vec<LabelTemplate>> = OnceLock::new();

fn templates() -> &'static [LabelTemplate] {
    TEMPLATES.get_or_init(|| label_match::parse_asset(ASSET))
}

/// Match a preprocessed (light-on-dark, canonically framed) grayscale type crop
/// against the embedded templates. Returns the best match at or above `MIN_NCC`.
pub fn match_type_label(crop: &[u8], width: usize, height: usize) -> Option<TypeMatch> {
    let m = label_match::best_match(templates(), crop, width, height, MIN_NCC)?;
    Some(TypeMatch {
        stype: StockpileType::from_discriminant(m.tag)?,
        lang: GameLanguage::from_discriminant(m.lang)?,
        score: m.score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_parses_into_all_type_labels() {
        // The embedded asset is well-formed and yields every rendered label
        // (12 types across the languages each supports = 62).
        let t = templates();
        assert_eq!(t.len(), 62);
        assert!(t
            .iter()
            .all(|t| StockpileType::from_discriminant(t.tag).is_some()));
        assert!(t
            .iter()
            .all(|t| GameLanguage::from_discriminant(t.lang).is_some()));
    }

    #[test]
    fn maps_discriminants_to_enums() {
        // Seaport (type 10) is in the set and maps back to the enum.
        let t = templates();
        assert!(t
            .iter()
            .any(|t| StockpileType::from_discriminant(t.tag) == Some(StockpileType::Seaport)));
    }
}

//! Public-default stockpile-name template matching.
//!
//! A public stockpile carries the game's localized default label
//! ("Public"/"Público"/"Öffentlich"/"Публичный"/"公共"), not a user-chosen name.
//! Like the type label it is a closed, game-rendered set, so matching it as a
//! template — rather than OCR + a per-language string dictionary — recognizes
//! the default in EVERY language, including Chinese/Russian whose custom names
//! the recognizer can't read. A hit means "this stockpile is public".

use std::sync::OnceLock;

use crate::enums::GameLanguage;
use crate::template::label_match::{self, LabelTemplate};

/// Embedded public-label asset (one render per language); see
/// `training/generate_type_templates.py`.
static ASSET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/public_templates.bin"
));

/// Minimum NCC for the name crop to count as the public default.
const MIN_NCC: f32 = 0.55;

static TEMPLATES: OnceLock<Vec<LabelTemplate>> = OnceLock::new();

fn templates() -> &'static [LabelTemplate] {
    TEMPLATES.get_or_init(|| label_match::parse_asset(ASSET))
}

/// If the preprocessed name crop matches a localized public default, return the
/// language it matched; otherwise `None` (the name is a custom one).
pub fn match_public_label(crop: &[u8], width: usize, height: usize) -> Option<GameLanguage> {
    let m = label_match::best_match(templates(), crop, width, height, MIN_NCC)?;
    // Public records store the language in `tag` (there is no per-type axis).
    GameLanguage::from_discriminant(m.tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_has_one_label_per_language() {
        // en, de, fr, pt, ru, zh — six public-default renders.
        let t = templates();
        assert_eq!(t.len(), 6);
        assert!(t
            .iter()
            .all(|t| GameLanguage::from_discriminant(t.tag).is_some()));
    }

    #[test]
    fn every_public_template_matches_itself() {
        // Each rendered public label self-matches. English and French are both
        // literally "Public" (pixel-identical), so their language is ambiguous —
        // only `is_some` ("this is public") is asserted across the board.
        for tpl in templates() {
            assert!(match_public_label(&tpl.pixels, tpl.width, tpl.height).is_some());
        }
    }

    #[test]
    fn distinctive_public_label_maps_to_its_language() {
        // German "Öffentlich" is unambiguous and must map to German.
        let de = templates()
            .iter()
            .find(|t| t.tag == GameLanguage::German as u8)
            .expect("german public label present");
        assert_eq!(
            match_public_label(&de.pixels, de.width, de.height),
            Some(GameLanguage::German)
        );
    }
}

//! In-binary stockpile-type label template matching.
//!
//! The stockpile type is a closed set of game-rendered strings — one per
//! language. This module embeds a clean render of each (generated offline by
//! `training/generate_type_templates.py`) and NCC-matches the live, preprocessed
//! type crop against them. A hit yields both the [`StockpileType`] and the
//! client's [`GameLanguage`], with no dependency on the OCR recognizer's
//! charset — so it covers Chinese/Russian even though those are dropped from the
//! recognizer.
//!
//! Reuses the icon match primitives ([`ncc_with_precomputed`]): the crop is
//! resized to each template's exact dimensions (NCC requires equal size), so a
//! template whose width is far from the crop's own distorts it and scores low.
//! That makes the match discriminative across the different-length labels.

use std::sync::OnceLock;

use crate::enums::{GameLanguage, StockpileType};
use crate::ocr::preprocess::upscale_bilinear;
use crate::template::matching::ncc_with_precomputed;

/// Embedded template asset; see `training/generate_type_templates.py` for the
/// binary format. Baked in at compile time, so there is no runtime file lookup.
static ASSET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/type_templates.bin"
));

/// Minimum NCC for a template to count as the type label. Below this the best
/// candidate is rejected and the caller falls back to OCR classification.
const MIN_NCC: f32 = 0.55;

/// One rendered type label with precomputed NCC statistics.
struct TypeTemplate {
    stype: StockpileType,
    lang: GameLanguage,
    width: usize,
    height: usize,
    pixels: Vec<u8>,
    ncc_mean: f32,
    ncc_inv_std: f32,
}

/// A successful type-label match.
#[derive(Debug, Clone, Copy)]
pub struct TypeMatch {
    pub stype: StockpileType,
    pub lang: GameLanguage,
    pub score: f32,
}

static TEMPLATES: OnceLock<Vec<TypeTemplate>> = OnceLock::new();

fn templates() -> &'static [TypeTemplate] {
    TEMPLATES.get_or_init(parse_asset)
}

/// NCC mean / inverse-std for a template, matching
/// `TemplateDatabase::compute_ncc_stats` so scores are on the same scale as the
/// icon matcher.
fn ncc_stats(pixels: &[u8]) -> (f32, f32) {
    if pixels.is_empty() {
        return (0.0, 0.0);
    }
    let n = pixels.len() as f32;
    let mean = pixels.iter().map(|&x| x as f32).sum::<f32>() / n;
    let var_sum: f32 = pixels.iter().map(|&x| (x as f32 - mean).powi(2)).sum();
    let std = var_sum.sqrt();
    let inv_std = if std > 1e-6 { 1.0 / std } else { 0.0 };
    (mean, inv_std)
}

/// Parse the embedded asset once. Each record is a grayscale PNG (decoded with
/// the `image` crate; the PNG header carries the dimensions). A malformed header
/// yields no templates (so matching is inert and the caller falls back to OCR);
/// a truncated or undecodable record stops/skips at that point.
fn parse_asset() -> Vec<TypeTemplate> {
    let data = ASSET;
    if data.len() < 12 || &data[0..4] != b"FSTT" {
        return Vec::new();
    }
    let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let mut out = Vec::with_capacity(count);
    let mut off = 12usize;
    for _ in 0..count {
        if off + 6 > data.len() {
            break;
        }
        let type_id = data[off];
        let lang_id = data[off + 1];
        let png_len =
            u32::from_le_bytes([data[off + 2], data[off + 3], data[off + 4], data[off + 5]])
                as usize;
        off += 6;
        if png_len == 0 || off + png_len > data.len() {
            break;
        }
        let png = &data[off..off + png_len];
        off += png_len;

        let (Some(stype), Some(lang)) = (
            StockpileType::from_discriminant(type_id),
            GameLanguage::from_discriminant(lang_id),
        ) else {
            continue;
        };
        let Ok(img) = image::load_from_memory(png) else {
            continue;
        };
        let gray = img.to_luma8();
        let (width, height) = (gray.width() as usize, gray.height() as usize);
        let pixels = gray.into_raw();
        let (ncc_mean, ncc_inv_std) = ncc_stats(&pixels);
        out.push(TypeTemplate {
            stype,
            lang,
            width,
            height,
            pixels,
            ncc_mean,
            ncc_inv_std,
        });
    }
    out
}

/// Match a preprocessed (light-on-dark, canonically framed) grayscale type crop
/// against the embedded templates. Returns the best match at or above
/// [`MIN_NCC`], or `None` when nothing clears the floor.
pub fn match_type_label(crop: &[u8], width: usize, height: usize) -> Option<TypeMatch> {
    if crop.is_empty() || width == 0 || height == 0 {
        return None;
    }

    let mut best: Option<TypeMatch> = None;
    let mut best_score = MIN_NCC;
    for t in templates() {
        if t.ncc_inv_std == 0.0 {
            continue;
        }
        let resized = upscale_bilinear(crop, width, height, t.width, t.height);
        let score = ncc_with_precomputed(&resized, &t.pixels, t.ncc_mean, t.ncc_inv_std);
        if score >= best_score {
            best_score = score;
            best = Some(TypeMatch {
                stype: t.stype,
                lang: t.lang,
                score,
            });
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_parses_into_templates() {
        // The embedded asset is well-formed and yields the expected count of
        // non-degenerate templates (all 62 rendered labels).
        let t = templates();
        assert_eq!(t.len(), 62);
        assert!(t.iter().all(|t| t.width > 0 && t.height == 64));
        assert!(t.iter().all(|t| t.pixels.len() == t.width * t.height));
        assert!(t.iter().all(|t| t.ncc_inv_std > 0.0));
    }

    #[test]
    fn a_template_matches_itself() {
        // Feeding a template's own pixels back must recover its type and language
        // with a near-perfect score — the canonical self-consistency check.
        let t = &templates()[0];
        let m = match_type_label(&t.pixels, t.width, t.height).expect("self match");
        assert_eq!(m.stype, t.stype);
        assert_eq!(m.lang, t.lang);
        assert!(m.score > 0.99, "self-match score {} too low", m.score);
    }

    #[test]
    fn blank_crop_does_not_match() {
        assert!(match_type_label(&[], 0, 0).is_none());
        let blank = vec![0u8; 100 * 64];
        assert!(match_type_label(&blank, 100, 64).is_none());
    }
}

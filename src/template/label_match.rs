//! Generic in-binary label template matching, shared by the stockpile-type
//! matcher ([`super::type_match`]) and the public-default-name matcher
//! ([`super::public_match`]).
//!
//! A label is a closed-set, game-rendered string. Each is rendered once offline
//! (`training/generate_type_templates.py`) into a grayscale PNG and embedded in
//! the binary. At match time the preprocessed (light-on-dark, canonically
//! framed) crop is resized to each template's exact dimensions — NCC requires
//! equal size — so a template whose width is far from the crop's own distorts it
//! and scores low, making the match discriminative across different-length
//! labels. Reuses the icon NCC primitive [`ncc_with_precomputed`].
//!
//! Asset format (little-endian), produced by the generator:
//! ```text
//! magic   b"FSTT"
//! version u32
//! count   u32
//! records[count]:
//!     tag     u8     (caller-defined: type_id, or lang_id for public)
//!     lang    u8     (caller-defined: lang_id, or 0)
//!     png_len u32
//!     png     png_len bytes (grayscale PNG; its header carries the dimensions)
//! ```

use crate::ocr::preprocess::upscale_bilinear;
use crate::template::matching::ncc_with_precomputed;

/// One rendered label template with precomputed NCC statistics. `tag`/`lang` are
/// opaque ids the caller maps to its own enums.
pub struct LabelTemplate {
    pub(crate) tag: u8,
    pub(crate) lang: u8,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<u8>,
    ncc_mean: f32,
    ncc_inv_std: f32,
}

/// The winning template's ids and NCC score.
#[derive(Debug, Clone, Copy)]
pub struct LabelMatch {
    pub tag: u8,
    pub lang: u8,
    pub score: f32,
}

/// NCC mean / inverse-std for a template, matching
/// `TemplateDatabase::compute_ncc_stats` so scores are on the icon matcher's
/// scale.
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

/// Parse a `FSTT` asset of PNG label records, decoding each PNG with the `image`
/// crate (its header carries the dimensions). A malformed header yields no
/// templates; a truncated or undecodable record stops/skips at that point.
pub fn parse_asset(data: &[u8]) -> Vec<LabelTemplate> {
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
        let tag = data[off];
        let lang = data[off + 1];
        let png_len =
            u32::from_le_bytes([data[off + 2], data[off + 3], data[off + 4], data[off + 5]])
                as usize;
        off += 6;
        if png_len == 0 || off + png_len > data.len() {
            break;
        }
        let png = &data[off..off + png_len];
        off += png_len;

        let Ok(img) = image::load_from_memory(png) else {
            continue;
        };
        let gray = img.to_luma8();
        let (width, height) = (gray.width() as usize, gray.height() as usize);
        let pixels = gray.into_raw();
        let (ncc_mean, ncc_inv_std) = ncc_stats(&pixels);
        out.push(LabelTemplate {
            tag,
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

/// Best NCC match at or above `min_ncc`, or `None` when nothing clears the floor.
pub fn best_match(
    templates: &[LabelTemplate],
    crop: &[u8],
    width: usize,
    height: usize,
    min_ncc: f32,
) -> Option<LabelMatch> {
    if crop.is_empty() || width == 0 || height == 0 {
        return None;
    }

    let mut best: Option<LabelMatch> = None;
    let mut best_score = min_ncc;
    for t in templates {
        if t.ncc_inv_std == 0.0 {
            continue;
        }
        let resized = upscale_bilinear(crop, width, height, t.width, t.height);
        let score = ncc_with_precomputed(&resized, &t.pixels, t.ncc_mean, t.ncc_inv_std);
        if score >= best_score {
            best_score = score;
            best = Some(LabelMatch {
                tag: t.tag,
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
    fn a_template_matches_itself() {
        // Feeding a type template's own pixels back recovers its ids at a
        // near-perfect score — exercises parse (PNG decode) + best_match.
        let templates = parse_asset(super::super::type_match::ASSET);
        assert!(!templates.is_empty());
        let t = &templates[0];
        let m = best_match(&templates, &t.pixels, t.width, t.height, 0.55).expect("self match");
        assert_eq!((m.tag, m.lang), (t.tag, t.lang));
        assert!(m.score > 0.99, "self-match score {} too low", m.score);
    }

    #[test]
    fn blank_and_empty_crops_do_not_match() {
        let templates = parse_asset(super::super::type_match::ASSET);
        assert!(best_match(&templates, &[], 0, 0, 0.55).is_none());
        let blank = vec![0u8; 100 * 64];
        assert!(best_match(&templates, &blank, 100, 64, 0.55).is_none());
    }

    #[test]
    fn malformed_asset_yields_no_templates() {
        assert!(parse_asset(b"").is_empty());
        assert!(parse_asset(b"NOPE\x00\x00\x00\x00\x00\x00\x00\x00").is_empty());
    }
}

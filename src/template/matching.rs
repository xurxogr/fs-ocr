//! NCC template matching.
//!
//! Implements Normalized Cross-Correlation (NCC) for template matching
//! using pure Rust for maximum performance.

use std::collections::HashSet;
use std::sync::Arc;

use rayon::prelude::*;

// Constants are now passed via config, but keep imports for default values in tests
use crate::enums::{ItemCategory, ItemFaction};
use crate::error::Result;

use super::database::{IconTemplate, TemplateDatabase};
use super::phash::filter_by_phash;

/// Filter options for template matching.
#[derive(Debug, Clone, Default)]
pub struct MatchFilter<'a> {
    /// Filter by faction.
    pub faction: Option<ItemFaction>,
    /// Filter by category.
    pub category: Option<ItemCategory>,
    /// Filter by crated status.
    pub crated: Option<bool>,
    /// Filter by mod name.
    pub mod_name: Option<&'a str>,
    /// Exclude specific item codes.
    pub excluded_codes: Option<&'a HashSet<String>>,
}

impl<'a> MatchFilter<'a> {
    /// Create an empty filter (no filtering).
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by faction.
    pub fn faction(mut self, faction: Option<ItemFaction>) -> Self {
        self.faction = faction;
        self
    }

    /// Filter by category.
    pub fn category(mut self, category: Option<ItemCategory>) -> Self {
        self.category = category;
        self
    }

    /// Filter by crated status.
    pub fn crated(mut self, crated: Option<bool>) -> Self {
        self.crated = crated;
        self
    }

    /// Filter by mod name.
    pub fn mod_name(mut self, mod_name: Option<&'a str>) -> Self {
        self.mod_name = mod_name;
        self
    }

    /// Exclude specific item codes.
    pub fn excluded_codes(mut self, excluded: Option<&'a HashSet<String>>) -> Self {
        self.excluded_codes = excluded;
        self
    }
}

/// Result of template matching for a single icon.
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Best matching template (if any).
    pub best_match: Option<IconTemplate>,
    /// Confidence of the best match (0.0 - 1.0).
    pub confidence: f64,
    /// Number of candidates tested with NCC.
    pub tested_candidates: usize,
    /// Top N matches with confidence scores.
    pub top_matches: Vec<(IconTemplate, f64)>,
    /// Alternative candidates within confidence gap.
    pub gap_candidates: Vec<(IconTemplate, f64)>,
}

impl MatchResult {
    /// Create an empty (no match) result.
    pub fn empty() -> Self {
        Self {
            best_match: None,
            confidence: 0.0,
            tested_candidates: 0,
            top_matches: Vec::new(),
            gap_candidates: Vec::new(),
        }
    }

    /// Check if a match was found.
    pub fn is_matched(&self) -> bool {
        self.best_match.is_some()
    }

    /// Get the matched code (or "Unknown").
    pub fn code(&self) -> &str {
        self.best_match
            .as_ref()
            .map(|t| t.code.as_str())
            .unwrap_or("Unknown")
    }
}

impl Default for MatchResult {
    fn default() -> Self {
        Self::empty()
    }
}

/// Template matcher using two-phase matching (pHash + NCC).
pub struct TemplateMatcher {
    /// Active template database.
    database: Arc<TemplateDatabase>,
    /// pHash threshold for filtering candidates.
    phash_threshold: u32,
    /// Hard cap on NCC candidates to evaluate (upper bound of escalation).
    max_ncc_candidates: usize,
    /// Confidence gap for returning alternatives.
    confidence_gap: f64,
    /// NCC tiebreaker threshold.
    ncc_tiebreaker_threshold: f64,
    /// Initial NCC batch size before escalation.
    ncc_initial_candidates: usize,
    /// Confidence floor below which the candidate count is escalated.
    ncc_escalation_threshold: f64,
    /// Number of top matches to keep.
    top_n: usize,
}

impl TemplateMatcher {
    /// Create a new template matcher with config parameters.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        database: Arc<TemplateDatabase>,
        phash_threshold: u32,
        max_ncc_candidates: usize,
        confidence_gap: f64,
        ncc_tiebreaker_threshold: f64,
        ncc_initial_candidates: usize,
        ncc_escalation_threshold: f64,
    ) -> Self {
        Self {
            database,
            phash_threshold,
            max_ncc_candidates,
            confidence_gap,
            ncc_tiebreaker_threshold,
            ncc_initial_candidates,
            ncc_escalation_threshold,
            top_n: 5,
        }
    }

    /// Match an icon with pre-computed pHash.
    pub fn match_icon_with_phash(
        &self,
        icon_image: &[u8],
        icon_width: i32,
        icon_height: i32,
        icon_phash: u64,
        filter: &MatchFilter,
    ) -> Result<MatchResult> {
        // Get filtered candidates
        let candidate_indices = self.database.get_candidates(
            filter.faction,
            filter.mod_name,
            filter.category,
            filter.crated,
            filter.excluded_codes,
        );

        if candidate_indices.is_empty() {
            return Ok(MatchResult::empty());
        }

        // Phase 1: pHash filtering
        let candidate_phashes: Vec<u64> = candidate_indices
            .iter()
            .map(|&i| self.database.phash_array[i])
            .collect();

        let phash_matches = filter_by_phash(
            icon_phash,
            &candidate_phashes,
            self.phash_threshold,
            self.max_ncc_candidates,
        );

        if phash_matches.is_empty() {
            return Ok(MatchResult::empty());
        }

        // Map back to database indices
        let ncc_candidates: Vec<usize> = phash_matches
            .iter()
            .map(|(local_idx, _)| candidate_indices[*local_idx])
            .collect();

        // Phase 2: NCC matching with adaptive candidate escalation.
        // Score an initial batch of the top-pHash candidates; only expand
        // (doubling, up to the pool size) when the best confidence stays below
        // the escalation threshold. Candidates are pHash-sorted, so each batch
        // adds the next-most-promising ones and earlier scores are reused.
        let pool_len = ncc_candidates.len();
        let mut all_matches: Vec<(usize, f64)> = Vec::with_capacity(pool_len);
        let mut scored = 0usize;
        let mut target = self.ncc_initial_candidates.clamp(1, pool_len);

        loop {
            let mut batch: Vec<(usize, f64)> = ncc_candidates[scored..target]
                .par_iter()
                .map(|&idx| {
                    let template = &self.database.templates[idx];
                    let template_mean = self.database.ncc_means[idx];
                    let template_inv_std = self.database.ncc_inv_stds[idx];

                    let confidence = if icon_image.len() == template.image_data.len() {
                        ncc_with_precomputed(
                            icon_image,
                            &template.image_data,
                            template_mean,
                            template_inv_std,
                        ) as f64
                    } else {
                        compute_ncc(
                            icon_image,
                            icon_width as usize,
                            icon_height as usize,
                            &template.image_data,
                            template.width as usize,
                            template.height as usize,
                        )
                    };
                    (idx, confidence)
                })
                .collect();
            all_matches.append(&mut batch);
            scored = target;

            let best_so_far = all_matches.iter().map(|&(_, c)| c).fold(f64::MIN, f64::max);

            // Stop once confident enough or the candidate pool is exhausted.
            if best_so_far >= self.ncc_escalation_threshold || scored >= pool_len {
                break;
            }
            target = (target * 2).min(pool_len);
        }

        // Sort by confidence (descending)
        all_matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if all_matches.is_empty() {
            return Ok(MatchResult::empty());
        }

        // Build result
        let (mut best_idx, mut best_confidence) = all_matches[0];

        // Apply tiebreaker if enabled and there are close matches
        if self.ncc_tiebreaker_threshold > 0.0 && all_matches.len() > 1 {
            // Find matches within tiebreaker threshold of best
            let close_matches: Vec<(usize, f64)> = all_matches
                .iter()
                .filter(|&&(_, conf)| best_confidence - conf <= self.ncc_tiebreaker_threshold)
                .copied()
                .collect();

            if close_matches.len() > 1 {
                // Get icon dimensions
                let width = icon_width as usize;
                let height = icon_height as usize;

                // Compute edge-based differences for close matches
                let mut scored: Vec<(f32, usize, f64)> = close_matches
                    .iter()
                    .map(|&(idx, conf)| {
                        let template = &self.database.templates[idx];
                        let edge_diff =
                            compute_edge_diff(icon_image, &template.image_data, width, height);
                        (edge_diff, idx, conf)
                    })
                    .collect();

                // Sort by edge diff (lower = better match)
                scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                // Update best match if tiebreaker changed the winner
                if !scored.is_empty() && scored[0].1 != best_idx {
                    best_idx = scored[0].1;
                    best_confidence = scored[0].2;
                }
            }
        }

        let best_template = self.database.templates[best_idx].clone();

        // Get top N matches
        let top_matches: Vec<(IconTemplate, f64)> = all_matches
            .iter()
            .take(self.top_n)
            .map(|&(idx, conf)| (self.database.templates[idx].clone(), conf))
            .collect();

        // Get gap candidates
        let gap_candidates: Vec<(IconTemplate, f64)> = if self.confidence_gap > 0.0 {
            all_matches
                .iter()
                .skip(1)
                .filter(|&(_, conf)| best_confidence - conf <= self.confidence_gap)
                .map(|&(idx, conf)| (self.database.templates[idx].clone(), conf))
                .collect()
        } else {
            Vec::new()
        };

        Ok(MatchResult {
            best_match: Some(best_template),
            confidence: best_confidence,
            tested_candidates: all_matches.len(),
            top_matches,
            gap_candidates,
        })
    }

    /// Get the template database.
    pub fn database(&self) -> &TemplateDatabase {
        &self.database
    }
}

/// Compute Normalized Cross-Correlation (NCC) between two images.
///
/// Pure Rust implementation for maximum performance.
/// Returns a value between -1.0 and 1.0, where 1.0 is a perfect match.
#[inline]
pub fn compute_ncc(
    image: &[u8],
    _image_width: usize,
    _image_height: usize,
    template: &[u8],
    _template_width: usize,
    _template_height: usize,
) -> f64 {
    // Fast path: same size images (most common case)
    if image.len() == template.len() && !image.is_empty() {
        return ncc_same_size(image, template);
    }

    // Size mismatch - return 0 (templates should be pre-scaled)
    0.0
}

/// Fast NCC for same-size images.
#[inline]
fn ncc_same_size(image: &[u8], template: &[u8]) -> f64 {
    let n = image.len() as f64;

    // Compute means using u64 to avoid overflow
    let icon_sum: u64 = image.iter().map(|&x| x as u64).sum();
    let template_sum: u64 = template.iter().map(|&x| x as u64).sum();
    let icon_mean = icon_sum as f64 / n;
    let template_mean = template_sum as f64 / n;

    // Compute NCC components in single pass
    let mut cross_sum = 0.0f64;
    let mut icon_var_sum = 0.0f64;
    let mut template_var_sum = 0.0f64;

    for (&i, &t) in image.iter().zip(template.iter()) {
        let icon_diff = i as f64 - icon_mean;
        let template_diff = t as f64 - template_mean;
        cross_sum += icon_diff * template_diff;
        icon_var_sum += icon_diff * icon_diff;
        template_var_sum += template_diff * template_diff;
    }

    let denominator = (icon_var_sum * template_var_sum).sqrt();
    if denominator < 1e-10 {
        return 0.0;
    }

    cross_sum / denominator
}

/// Fast NCC with precomputed template statistics.
///
/// This version uses precomputed template mean and inverse std for ~2x speedup.
/// Only computes icon statistics at runtime.
#[inline]
pub fn ncc_with_precomputed(
    image: &[u8],
    template: &[u8],
    template_mean: f32,
    template_inv_std: f32,
) -> f32 {
    if image.len() != template.len() || image.is_empty() || template_inv_std == 0.0 {
        return 0.0;
    }

    // Use scalar version - LLVM auto-vectorizes better than manual SIMD
    ncc_with_precomputed_scalar(image, template, template_mean, template_inv_std)
}

/// Scalar fallback for NCC computation.
#[inline]
fn ncc_with_precomputed_scalar(
    image: &[u8],
    template: &[u8],
    template_mean: f32,
    template_inv_std: f32,
) -> f32 {
    let n = image.len() as f32;

    // Compute icon mean
    let icon_sum: u32 = image.iter().map(|&x| x as u32).sum();
    let icon_mean = icon_sum as f32 / n;

    // Single pass: compute cross-correlation and icon variance
    let mut cross_sum = 0.0f32;
    let mut icon_var_sum = 0.0f32;

    for (&i, &t) in image.iter().zip(template.iter()) {
        let icon_diff = i as f32 - icon_mean;
        let template_diff = t as f32 - template_mean;
        cross_sum += icon_diff * template_diff;
        icon_var_sum += icon_diff * icon_diff;
    }

    let icon_std = icon_var_sum.sqrt();
    if icon_std < 1e-6 {
        return 0.0;
    }

    // NCC = cross / (icon_std * template_std) = cross * template_inv_std / icon_std
    cross_sum * template_inv_std / icon_std
}

/// Compute Sobel mixed derivative for a grayscale image.
///
/// Matches cv2.Sobel(img, cv2.CV_32F, 1, 1) which computes d²f/dxdy.
/// The kernel for mixed derivative is: [1, 0, -1; 0, 0, 0; -1, 0, 1]
/// The image is assumed to be in row-major order with 3 channels (BGR).
#[inline]
fn compute_edge_magnitude(image: &[u8], width: usize, height: usize) -> Vec<f32> {
    // Convert to grayscale first
    let gray: Vec<u8> = image
        .chunks_exact(3)
        .map(|rgb| {
            // Standard grayscale conversion: 0.299*R + 0.587*G + 0.114*B
            // BGR order: B=0, G=1, R=2
            ((rgb[2] as f32 * 0.299) + (rgb[1] as f32 * 0.587) + (rgb[0] as f32 * 0.114)) as u8
        })
        .collect();

    // Mixed derivative kernel (cv2.Sobel with dx=1, dy=1, ksize=3)
    // Kernel: [1, 0, -1; 0, 0, 0; -1, 0, 1]
    let mut edges = vec![0.0f32; width * height];

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let idx = y * width + x;

            // Get corner pixels for mixed derivative
            let p00 = gray[(y - 1) * width + (x - 1)] as f32;
            let p02 = gray[(y - 1) * width + (x + 1)] as f32;
            let p20 = gray[(y + 1) * width + (x - 1)] as f32;
            let p22 = gray[(y + 1) * width + (x + 1)] as f32;

            // Mixed derivative: d²f/dxdy
            // Kernel: [1, 0, -1; 0, 0, 0; -1, 0, 1]
            edges[idx] = p00 - p02 - p20 + p22;
        }
    }

    edges
}

/// Compute mean absolute edge difference between two images.
#[inline]
fn compute_edge_diff(image1: &[u8], image2: &[u8], width: usize, height: usize) -> f32 {
    let edges1 = compute_edge_magnitude(image1, width, height);
    let edges2 = compute_edge_magnitude(image2, width, height);

    let sum: f32 = edges1
        .iter()
        .zip(edges2.iter())
        .map(|(e1, e2)| (e1 - e2).abs())
        .sum();

    sum / edges1.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_match_result() {
        let result = MatchResult::empty();
        assert!(!result.is_matched());
        assert_eq!(result.code(), "Unknown");
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_ncc_identical_images() {
        // Use a non-uniform image (gradient) for meaningful NCC
        let image: Vec<u8> = (0..64 * 64 * 3).map(|i| (i % 256) as u8).collect();
        let ncc = compute_ncc(&image, 64, 64, &image, 64, 64);
        assert!(
            (ncc - 1.0).abs() < 0.001,
            "NCC should be 1.0 for identical images, got {}",
            ncc
        );
    }

    #[test]
    fn test_ncc_inverse_images() {
        let image1: Vec<u8> = (0..64 * 64 * 3).map(|i| (i % 256) as u8).collect();
        let image2: Vec<u8> = image1.iter().map(|&x| 255 - x).collect();

        let ncc = compute_ncc(&image1, 64, 64, &image2, 64, 64);
        assert!(ncc < 0.0, "NCC should be negative for inverse images");
    }

    #[test]
    fn test_ncc_similar_images() {
        let mut image1: Vec<u8> = vec![128u8; 64 * 64 * 3];
        let mut image2 = image1.clone();

        // Add some variation
        for i in 0..100 {
            image1[i] = 130;
            image2[i] = 132;
        }

        let ncc = compute_ncc(&image1, 64, 64, &image2, 64, 64);
        assert!(ncc > 0.9, "NCC should be high for similar images: {}", ncc);
    }

    #[test]
    fn test_ncc_different_sizes_returns_zero() {
        let image1: Vec<u8> = vec![128u8; 64 * 64 * 3];
        let image2: Vec<u8> = vec![128u8; 32 * 32 * 3];

        // Different sizes now return 0 (templates should be pre-scaled)
        let ncc = compute_ncc(&image1, 64, 64, &image2, 32, 32);
        assert_eq!(ncc, 0.0);
    }
}

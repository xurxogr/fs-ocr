//! Hardcoded constants for the OCR pipeline.
//!
//! All layout values are based on 2160p resolution and scale proportionally.

/// Base resolution height for all layout calculations.
pub const BASE_RESOLUTION: i32 = 2160;

// =============================================================================
// Layout Detection (scale with resolution)
// =============================================================================

/// Width of quantity boxes at 2160p.
pub const BOX_WIDTH: i32 = 84;

/// Height of quantity boxes at 2160p.
pub const BOX_HEIGHT: i32 = 64;

/// Horizontal spacing between quantity boxes at 2160p.
pub const COLUMN_OFFSET: i32 = 112;

/// Vertical spacing between rows at 2160p.
pub const ROW_OFFSET: i32 = 78;

/// Vertical spacing between groups at 2160p.
pub const GROUP_OFFSET: i32 = 98;

/// Horizontal offset from icon to quantity box at 2160p.
pub const ICON_TO_QUANTITY_OFFSET: i32 = 88;

/// Margin from first icon to title at 2160p.
pub const TITLE_MARGIN: i32 = 24;

/// Minimum title region width at 2160p.
pub const TITLE_MIN_WIDTH: i32 = 600;

/// Title region height at 2160p.
pub const TITLE_HEIGHT: i32 = 64;

// =============================================================================
// Grey Detection Thresholds
// =============================================================================

/// Minimum grey value (HSV/RGB).
pub const GRAY_LOWER: u8 = 15;

/// Maximum grey value (HSV/RGB).
pub const GRAY_UPPER: u8 = 98;

/// Pixel coordinate tolerance for box alignment.
pub const PIXEL_DIFF_TOLERANCE: i32 = 2;

/// Base sample rate for row scanning at 2160p (scales with resolution).
pub const SAMPLE_RATE_BASE: i32 = 10;

// =============================================================================
// Template Matching
// =============================================================================

/// Maximum Hamming distance for pHash filtering.
/// Lower value = fewer candidates, faster matching.
/// Note: 12 was too aggressive - some icons need threshold 15 to match correctly.
pub const PHASH_THRESHOLD: u32 = 15;

/// Hard cap on candidates evaluated with NCC after pHash filtering.
/// This is the upper bound of adaptive escalation: matching starts with
/// `NCC_INITIAL_CANDIDATES` and only expands toward this cap when the best
/// confidence stays below `NCC_ESCALATION_THRESHOLD`. The common case stops
/// at the initial batch, so raising the cap costs nothing on easy icons while
/// letting hard ones (e.g. modded RifleW/StickyBomb) reach the right template.
pub const MAX_NCC_CANDIDATES: usize = 100;

/// Initial NCC batch size for adaptive escalation.
/// The first matching attempt only scores this many top-pHash candidates.
pub const NCC_INITIAL_CANDIDATES: usize = 25;

/// Confidence floor for adaptive escalation.
/// If the best NCC confidence after a batch is below this value, the candidate
/// count is doubled (up to `MAX_NCC_CANDIDATES`) and matching retries. Reusing
/// already-computed scores keeps escalation cheap.
///
/// Calibrated from the reference (fs) dataset (36,730 matches): 93.9% of correct
/// matches score >= 0.95, with a thin ambiguous tail below 0.90. 0.90 sits in the
/// valley between them, so easy icons never escalate while borderline matches
/// (where a wrong template can win within the first batch) always get a second
/// look. Raising it only costs extra NCC work; it never hurts correctness.
pub const NCC_ESCALATION_THRESHOLD: f64 = 0.90;

/// NCC tiebreaker threshold.
/// When top matches are within this threshold, use edge-based (Sobel) comparison
/// to pick the winner. Set to 0.0 to disable.
///
/// Slightly wider than the reference (fs) 0.002: some genuine near-ties have an
/// NCC gap just over 0.002 (e.g. the CDW-A ExplosiveLightC vs RifleAutomaticC
/// pair sits at ~0.0029), where the raw-NCC winner is wrong but the edge-diff
/// comparison is correct. 0.003 lets the tiebreaker fire on those.
pub const NCC_TIEBREAKER_THRESHOLD: f64 = 0.003;

/// Width of stockpile type region (4x box_width at 2160p).
pub const STOCKPILE_TYPE_WIDTH_FACTOR: f64 = 4.0;

/// Vertical placement of the type label inside the box-height band that sits
/// just above the ROI. Measured across 1050p–2160p: the text consistently
/// occupies ~28%–82% of the band, with a noise strip above and grey background
/// below. Cropping to this slab directly yields a tight, text-only region — no
/// runtime band-finding needed.
pub const STOCKPILE_TYPE_TOP_FACTOR: f64 = 0.24;
/// Height of the type label as a fraction of the box-height band (covers the
/// ~46% text run plus a margin on each side, with extra headroom above so the
/// taller old-format name line — which shares this slab — isn't read tight).
pub const STOCKPILE_TYPE_HEIGHT_FACTOR: f64 = 0.64;

/// Width of stockpile name region (2.5x box_width at 2160p).
pub const STOCKPILE_NAME_WIDTH_FACTOR: f64 = 2.5;

/// Shard width factor (3.5x box_width at 2160p).
pub const SHARD_WIDTH_FACTOR: f64 = 3.5;

// =============================================================================
// Supported Resolutions
// =============================================================================

/// All supported vertical resolutions for template matching.
/// These must match the resolution groups in the HDF5 database.
pub const SUPPORTED_RESOLUTIONS: [i32; 16] = [
    664, 720, 768, 800, 864, 900, 960, 992, 1024, 1050, 1080, 1200, 1440, 1536, 1600, 2160,
];

/// Returns the closest supported resolution for a given height.
pub fn find_closest_resolution(height: i32) -> i32 {
    SUPPORTED_RESOLUTIONS
        .iter()
        .min_by_key(|&&r| (r - height).abs())
        .copied()
        .unwrap_or(2160)
}

/// Computes the scale factor for a given resolution.
#[inline]
pub fn compute_scale_factor(height: i32) -> f64 {
    height as f64 / BASE_RESOLUTION as f64
}

/// Scales a value from base resolution to target resolution.
///
/// Uses truncation (floor for positive values) to match Python's int() behavior.
#[inline]
pub fn scale_value(value: i32, scale_factor: f64) -> i32 {
    (value as f64 * scale_factor) as i32
}

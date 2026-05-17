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

/// Margin for adaptive grey thresholding.
pub const ADAPTIVE_MARGIN: u8 = 5;

/// Base sample rate for row scanning at 2160p (scales with resolution).
pub const SAMPLE_RATE_BASE: i32 = 10;

// =============================================================================
// Template Matching
// =============================================================================

/// Maximum Hamming distance for pHash filtering.
/// Lower value = fewer candidates, faster matching.
/// Note: 12 was too aggressive - some icons need threshold 15 to match correctly.
pub const PHASH_THRESHOLD: u32 = 15;

/// Maximum candidates to evaluate with NCC after pHash filtering.
/// Note: 25 was too few - some icons need more candidates to find correct match.
/// Reduced from 50 to 30 for better performance with acceptable accuracy.
pub const MAX_NCC_CANDIDATES: usize = 30;

/// NCC tiebreaker threshold.
/// When top matches are within this threshold, use edge-based comparison.
/// Set to 0.0 to disable tiebreaker.
pub const NCC_TIEBREAKER_THRESHOLD: f64 = 0.0015;

// =============================================================================
// Morphological Kernel Sizes
// =============================================================================

/// Close kernel size (fills small gaps).
pub const CLOSE_KERNEL_SIZE: i32 = 3;

/// Open kernel size (separates merged boxes).
pub const OPEN_KERNEL_SIZE: i32 = 5;

// =============================================================================
// OCR Configuration
// =============================================================================

/// Default upscale factor for quantity OCR.
pub const QUANTITY_UPSCALE_FACTOR: f64 = 2.0;

/// Upscale factor for text regions (type, name, etc.).
pub const TEXT_UPSCALE_FACTOR: f64 = 4.0;

/// Minimum standard deviation for tab button contrast detection.
pub const TAB_CONTRAST_THRESHOLD: f64 = 30.0;

// =============================================================================
// Number of columns in stockpile grid
// =============================================================================

/// Number of columns in the stockpile grid.
pub const GRID_COLUMNS: usize = 6;

/// Width of stockpile type region (4x box_width at 2160p).
pub const STOCKPILE_TYPE_WIDTH_FACTOR: f64 = 4.0;

/// Width of stockpile name region (2.5x box_width at 2160p).
pub const STOCKPILE_NAME_WIDTH_FACTOR: f64 = 2.5;

/// Shard width factor (3.5x box_width at 2160p).
pub const SHARD_WIDTH_FACTOR: f64 = 3.5;

// =============================================================================
// Info Bar Heights (for stockpile type detection)
// =============================================================================

/// Height of the grey separator bar at 2160p.
pub const GREY_BAR_HEIGHT: i32 = 6;

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

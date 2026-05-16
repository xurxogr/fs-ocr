//! Black box detection for stockpile region localization.
//!
//! Detects dark/black rectangular areas that contain item icons.
//! Used as a fast first pass to identify the Region of Interest (ROI)
//! before running detailed grey mask detection on the cropped region.

use crate::constants::{
    compute_scale_factor, scale_value, BOX_HEIGHT, PIXEL_DIFF_TOLERANCE, ROW_OFFSET,
};

use crate::error::Result;

use super::geometry::BoundingRect;

/// Threshold for dark pixel detection (RGB values 0-15).
const BLACK_THRESHOLD: u8 = 15;

/// Minimum width for black box at 2160p base resolution.
const MIN_WIDTH_2160: i32 = 600;

/// Maximum width for black box at 2160p base resolution.
const MAX_WIDTH_2160: i32 = 1200;

/// Valid stockpile widths at 2160p base resolution.
/// At 1080p these scale to: 300, 306, 404, 502, 600
/// The pattern is: base widths (600, 612) then +2*GROUP_OFFSET for each additional column.
const VALID_WIDTHS_2160: [i32; 5] = [600, 612, 808, 1004, 1200];

/// Result of black box detection.
#[derive(Debug, Clone)]
pub struct BlackBoxResult {
    /// Bounding region of the detected stockpile area (with padding).
    pub roi: BoundingRect,
    /// Scale factor used for detection.
    pub scale_factor: f64,
}

/// A horizontal run of black pixels.
#[derive(Debug, Clone, Copy)]
struct BlackRun {
    y: i32,
    x: i32,
    width: i32,
}

/// Detector for black icon areas to find stockpile region.
pub struct BlackBoxDetector {
    /// Scale factor relative to base resolution.
    scale_factor: f64,
    /// Image width.
    image_width: i32,
    /// Image height.
    image_height: i32,
    /// Minimum expected width (600px at 2160p, scaled).
    min_width: i32,
    /// Maximum expected width (1200px at 2160p, scaled).
    max_width: i32,
    /// Target height (row_gap + row_offset).
    target_height: i32,
    /// Sample rate for sparse row scanning (scaled with resolution).
    sample_rate: usize,
}

impl BlackBoxDetector {
    /// Create a new black box detector for the given image dimensions.
    ///
    /// Detection strategy using row-by-row scanning:
    /// - Scan each row for horizontal black runs of valid width (600-1200px at 2160p)
    /// - Group consecutive rows with runs at similar X positions
    /// - Filter groups by height (row_gap + row_offset ± tolerance)
    /// - Select topmost valid rectangle
    pub fn new(image_width: i32, image_height: i32) -> Self {
        let scale_factor = compute_scale_factor(image_height);

        // At 2160p: ROW_OFFSET = 78px, BOX_HEIGHT = 64px
        let box_height = scale_value(BOX_HEIGHT, scale_factor);
        let row_offset = scale_value(ROW_OFFSET, scale_factor);

        // row_gap = ROW_OFFSET - BOX_HEIGHT = 14px at 2160p
        let row_gap = row_offset - box_height;

        // Width: 600-1200px at 2160p, scaled with PIXEL_DIFF_TOLERANCE
        let min_width = scale_value(MIN_WIDTH_2160, scale_factor) - PIXEL_DIFF_TOLERANCE;
        let max_width = scale_value(MAX_WIDTH_2160, scale_factor) + PIXEL_DIFF_TOLERANCE;

        // Target height: row_gap + row_offset = 92px at 2160p
        let target_height = row_gap + row_offset;

        // Sample rate: 10 at 2160p, scaled down for lower resolutions (min 5)
        let sample_rate = ((10.0 * scale_factor) as usize).max(5);

        Self {
            scale_factor,
            image_width,
            image_height,
            min_width,
            max_width,
            target_height,
            sample_rate,
        }
    }

    /// Get the scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Get filter constraints: (scale_factor, min_width, max_width, min_height).
    pub fn get_constraints(&self) -> (f64, i32, i32, i32) {
        (
            self.scale_factor,
            self.min_width,
            self.max_width,
            self.target_height - PIXEL_DIFF_TOLERANCE,
        )
    }

    /// Detect the stockpile region using sparse row sampling.
    ///
    /// Algorithm:
    /// 1. Sample every Nth row for 600-1200px black runs (N = sample_rate)
    /// 2. Group runs by X overlap/adjacency (handles multi-column stockpiles)
    /// 3. Pick the group with most runs
    /// 4. Extend left/right from middle row until non-black
    /// 5. Extend up/down until non-black
    ///
    /// Returns the bounding region (ROI) that can be used to crop the image
    /// before running detailed grey mask detection.
    pub fn detect(&self, image: &[u8], width: i32, height: i32) -> Result<Option<BlackBoxResult>> {
        let w = width as usize;
        let h = height as usize;

        // Step 1: Sample rows for valid black runs (600-1200px at 2160p, scaled)
        let mut all_runs: Vec<BlackRun> = Vec::new();

        for y in (0..h).step_by(self.sample_rate) {
            let mut in_run = false;
            let mut run_start = 0usize;

            for x in 0..w {
                let idx = (y * w + x) * 3;
                let is_black = image[idx] < BLACK_THRESHOLD
                    && image[idx + 1] < BLACK_THRESHOLD
                    && image[idx + 2] < BLACK_THRESHOLD;

                if is_black && !in_run {
                    in_run = true;
                    run_start = x;
                } else if !is_black && in_run {
                    let run_len = (x - run_start) as i32;
                    if run_len >= self.min_width && run_len <= self.max_width {
                        all_runs.push(BlackRun {
                            y: y as i32,
                            x: run_start as i32,
                            width: run_len,
                        });
                    }
                    in_run = false;
                }
            }

            if in_run {
                let run_len = (w - run_start) as i32;
                if run_len >= self.min_width && run_len <= self.max_width {
                    all_runs.push(BlackRun {
                        y: y as i32,
                        x: run_start as i32,
                        width: run_len,
                    });
                }
            }
        }

        if all_runs.is_empty() {
            return Ok(None);
        }

        // Step 2: Group runs by X overlap/adjacency (merges multi-column stockpiles)
        // Tolerance of 200px accounts for icons/gaps between columns
        // IMPORTANT: Compare against individual runs, not the group bounding box,
        // to prevent "chaining" where distant runs get grouped through intermediates.
        const X_ADJACENCY_TOLERANCE: i32 = 200;
        let mut groups: Vec<Vec<BlackRun>> = Vec::new();

        for run in all_runs {
            let mut found_idx = None;
            for (idx, group) in groups.iter().enumerate() {
                // Check if this run overlaps/adjacent to ANY individual run in the group
                let is_adjacent = group.iter().any(|existing| {
                    let existing_end = existing.x + existing.width;
                    let run_end = run.x + run.width;

                    // Runs are adjacent if they overlap or are within tolerance
                    run.x <= existing_end + X_ADJACENCY_TOLERANCE
                        && run_end >= existing.x - X_ADJACENCY_TOLERANCE
                });

                if is_adjacent {
                    found_idx = Some(idx);
                    break;
                }
            }

            match found_idx {
                Some(idx) => groups[idx].push(run),
                None => groups.push(vec![run]),
            }
        }

        // Step 3: Pick the group with most runs (this is the stockpile)
        let best_group = match groups.iter().max_by_key(|g| g.len()) {
            Some(g) => g,
            None => return Ok(None),
        };

        // Compute bounding box from all runs in the group
        let mut roi_x = best_group.iter().map(|r| r.x).min().unwrap();
        let roi_x_end = best_group.iter().map(|r| r.x + r.width).max().unwrap();
        let first_bar_y = best_group.iter().map(|r| r.y).min().unwrap();
        let last_bar_y = best_group.iter().map(|r| r.y).max().unwrap();
        let mut roi_w = roi_x_end - roi_x;

        // Step 4: Extend left/right from middle row until non-black
        let mid_y = ((first_bar_y + last_bar_y) / 2) as usize;

        // Extend LEFT
        for x in (0..roi_x as usize).rev() {
            let idx = (mid_y * w + x) * 3;
            if image[idx] < BLACK_THRESHOLD
                && image[idx + 1] < BLACK_THRESHOLD
                && image[idx + 2] < BLACK_THRESHOLD
            {
                roi_x = x as i32;
            } else {
                break;
            }
        }

        // Extend RIGHT
        for x in (roi_x_end as usize)..w {
            let idx = (mid_y * w + x) * 3;
            if image[idx] < BLACK_THRESHOLD
                && image[idx + 1] < BLACK_THRESHOLD
                && image[idx + 2] < BLACK_THRESHOLD
            {
                roi_w = (x as i32 + 1) - roi_x;
            } else {
                break;
            }
        }

        // Step 5: Extend UP from first bar until non-black
        // Allow small gaps (info bar separators) of up to 6px at 2160p, min 3px
        let max_gap = scale_value(6, self.scale_factor).max(3) as usize;
        let check_x = (roi_x + roi_w / 2) as usize;
        let mut y_start = first_bar_y;
        let mut gap_count = 0usize;
        for check_y in (0..first_bar_y as usize).rev() {
            if check_x >= w {
                break;
            }
            let idx = (check_y * w + check_x) * 3;
            let is_black = image[idx] < BLACK_THRESHOLD
                && image[idx + 1] < BLACK_THRESHOLD
                && image[idx + 2] < BLACK_THRESHOLD;

            if is_black {
                y_start = check_y as i32;
                gap_count = 0;
            } else {
                gap_count += 1;
                if gap_count > max_gap {
                    break;
                }
            }
        }

        // Step 6: Extend DOWN from last bar until non-black
        let mut y_end = last_bar_y;
        let mut gap_count = 0usize;
        for check_y in (last_bar_y as usize + 1)..h {
            if check_x >= w {
                break;
            }
            let idx = (check_y * w + check_x) * 3;
            let is_black = image[idx] < BLACK_THRESHOLD
                && image[idx + 1] < BLACK_THRESHOLD
                && image[idx + 2] < BLACK_THRESHOLD;

            if is_black {
                y_end = check_y as i32;
                gap_count = 0;
            } else {
                gap_count += 1;
                if gap_count > max_gap {
                    break;
                }
            }
        }

        let roi_h = y_end - y_start;

        // Minimum height check (30px at 2160p, scaled)
        let min_height = scale_value(30, self.scale_factor);
        if roi_h < min_height {
            return Ok(None);
        }

        // Snap width to valid stockpile width
        let snapped_width = self.snap_to_valid_width(roi_w);
        let final_roi_w = snapped_width.min(self.image_width - roi_x);
        let final_roi_h = roi_h.min(self.image_height - y_start);

        let roi = (roi_x.max(0), y_start.max(0), final_roi_w, final_roi_h);

        Ok(Some(BlackBoxResult {
            roi,
            scale_factor: self.scale_factor,
        }))
    }

    /// Snap a detected width to the nearest valid stockpile width.
    ///
    /// Valid widths follow the pattern: base widths (600, 612) then +2*GROUP_OFFSET
    /// for each additional column at 2160p. These are scaled for other resolutions.
    fn snap_to_valid_width(&self, raw_width: i32) -> i32 {
        // Scale valid widths from 2160p to current resolution
        let valid_widths: Vec<i32> = VALID_WIDTHS_2160
            .iter()
            .map(|&w| scale_value(w, self.scale_factor))
            .collect();

        // Find nearest valid width (always snap up to not cut off boxes)
        for &valid in &valid_widths {
            if raw_width <= valid {
                return valid;
            }
        }

        // If larger than max, use the detected width
        raw_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detector_creation() {
        let detector = BlackBoxDetector::new(3840, 2160);
        assert!((detector.scale_factor - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detector_scaling_1080p() {
        let detector = BlackBoxDetector::new(1920, 1080);
        assert!((detector.scale_factor - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_no_detection_on_empty() {
        let detector = BlackBoxDetector::new(100, 100);
        let image = vec![128u8; 100 * 100 * 3]; // All grey

        let result = detector.detect(&image, 100, 100).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_width_constraints() {
        // At 2160p, width should be 598-1202 (600-1200 ± 2)
        let detector = BlackBoxDetector::new(3840, 2160);
        assert_eq!(detector.min_width, 598);
        assert_eq!(detector.max_width, 1202);

        // At 1080p, width should be 298-602 (scaled by 0.5)
        let detector = BlackBoxDetector::new(1920, 1080);
        assert_eq!(detector.min_width, 298);
        assert_eq!(detector.max_width, 602);
    }

    #[test]
    fn test_width_snapping_1080p() {
        // Valid widths at 1080p: 300, 306, 404, 502, 600
        let detector = BlackBoxDetector::new(1920, 1080);

        // Exact matches
        assert_eq!(detector.snap_to_valid_width(300), 300);
        assert_eq!(detector.snap_to_valid_width(306), 306);
        assert_eq!(detector.snap_to_valid_width(404), 404);
        assert_eq!(detector.snap_to_valid_width(502), 502);
        assert_eq!(detector.snap_to_valid_width(600), 600);

        // Values below should snap up
        assert_eq!(detector.snap_to_valid_width(280), 300);
        assert_eq!(detector.snap_to_valid_width(350), 404);
        assert_eq!(detector.snap_to_valid_width(584), 600); // Key test case

        // Values above max should remain as-is
        assert_eq!(detector.snap_to_valid_width(650), 650);
    }

    #[test]
    fn test_width_snapping_2160p() {
        // Valid widths at 2160p: 600, 612, 808, 1004, 1200
        let detector = BlackBoxDetector::new(3840, 2160);

        // Exact matches
        assert_eq!(detector.snap_to_valid_width(600), 600);
        assert_eq!(detector.snap_to_valid_width(612), 612);
        assert_eq!(detector.snap_to_valid_width(808), 808);
        assert_eq!(detector.snap_to_valid_width(1004), 1004);
        assert_eq!(detector.snap_to_valid_width(1200), 1200);

        // Values below should snap up
        assert_eq!(detector.snap_to_valid_width(580), 600);
        assert_eq!(detector.snap_to_valid_width(700), 808);
        assert_eq!(detector.snap_to_valid_width(1168), 1200); // Key test case (2x 584)
    }
}

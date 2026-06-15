//! Grey mask detection for stockpile quantity boxes.
//!
//! Uses a dual-mask approach combining HSV and RGB color space
//! for robust grey detection across different lighting conditions.

mod grouping;
mod morphology;

use rayon::prelude::*;

use morphology::{dilate, erode, find_contours};

use crate::constants::{
    compute_scale_factor, scale_value, BOX_HEIGHT, BOX_WIDTH, COLUMN_OFFSET, GRAY_LOWER,
    GRAY_UPPER, GROUP_OFFSET, ICON_TO_QUANTITY_OFFSET, ROW_OFFSET, SAMPLE_RATE_BASE,
    SHARD_WIDTH_FACTOR, STOCKPILE_NAME_WIDTH_FACTOR, STOCKPILE_TYPE_HEIGHT_FACTOR,
    STOCKPILE_TYPE_TOP_FACTOR, STOCKPILE_TYPE_WIDTH_FACTOR, TITLE_HEIGHT, TITLE_MARGIN,
    TITLE_MIN_WIDTH,
};
use crate::error::{FsOcrError, Result};

use super::geometry::{BoundingRect, Coordinates, DetectedRegions, GroupInfo};

/// Detector for grey quantity boxes in stockpile screenshots.
pub struct GreyMaskDetector {
    /// Scale factor relative to base resolution.
    scale_factor: f64,
    /// Image height.
    image_height: i32,
    /// Scaled box width.
    box_width: i32,
    /// Scaled box height (also used as icon size since icons are box_height x box_height).
    box_height: i32,
    /// Scaled column offset.
    column_offset: i32,
    /// Scaled row offset.
    row_offset: i32,
    /// Scaled group offset.
    group_offset: i32,
    /// Lower grey threshold.
    gray_lower: u8,
    /// Upper grey threshold.
    gray_upper: u8,
    /// Title margin.
    title_margin: i32,
    /// Title minimum width.
    title_min_width: i32,
    /// Title height.
    title_height: i32,
    /// Stockpile type width.
    stockpile_type_width: i32,
    /// Stockpile name width.
    stockpile_name_width: i32,
    /// Shard width.
    shard_width: i32,
}

impl GreyMaskDetector {
    /// Create a new grey mask detector for the given image dimensions.
    pub fn new(_image_width: i32, image_height: i32) -> Self {
        let scale_factor = compute_scale_factor(image_height);
        let box_width = scale_value(BOX_WIDTH, scale_factor);
        let box_height = scale_value(BOX_HEIGHT, scale_factor);

        // Note: Python uses (COLUMN_OFFSET + BOX_WIDTH) for column_offset
        // This is intentional to match the grid spacing calculation
        Self {
            scale_factor,
            image_height,
            box_width,
            box_height,
            column_offset: scale_value(COLUMN_OFFSET + BOX_WIDTH, scale_factor),
            row_offset: scale_value(ROW_OFFSET, scale_factor),
            group_offset: scale_value(GROUP_OFFSET, scale_factor),
            gray_lower: GRAY_LOWER,
            gray_upper: GRAY_UPPER,
            title_margin: scale_value(TITLE_MARGIN, scale_factor),
            title_min_width: scale_value(TITLE_MIN_WIDTH, scale_factor),
            title_height: scale_value(TITLE_HEIGHT, scale_factor),
            stockpile_type_width: (box_width as f64 * STOCKPILE_TYPE_WIDTH_FACTOR) as i32,
            stockpile_name_width: (box_width as f64 * STOCKPILE_NAME_WIDTH_FACTOR) as i32,
            shard_width: (box_width as f64 * SHARD_WIDTH_FACTOR) as i32,
        }
    }

    /// Get the scale factor.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Detect quantity boxes in an RGB image.
    ///
    /// Args:
    ///     image: RGB image data (row-major, 3 bytes per pixel)
    ///     width: Image width
    ///     height: Image height
    ///
    /// Returns:
    ///     DetectedRegions containing all detected boxes and groups
    pub fn detect(&self, image: &[u8], width: i32, height: i32) -> Result<DetectedRegions> {
        self.detect_internal(image, width, height, true)
    }

    /// Fast ROI detection using adaptive grey threshold.
    ///
    /// Detects boxes by finding the most common "black" color in the first row,
    /// then using (black + gap) as threshold to detect quantity boxes.
    /// No morphology needed - the threshold keeps boxes separated.
    #[allow(clippy::too_many_arguments)]
    pub fn detect_roi_fast(
        &self,
        image: &[u8],
        img_width: i32,
        _img_height: i32,
        roi_x: i32,
        roi_y: i32,
        roi_w: i32,
        roi_h: i32,
    ) -> Result<DetectedRegions> {
        let iw = img_width as usize;
        let rx = roi_x as usize;
        let ry = roi_y as usize;
        let rw = roi_w as usize;
        let rh = roi_h as usize;

        // Sample horizontal rows to find grey box values (low chroma pixels)
        // Icons are colored (high chroma), boxes are grey (low chroma)
        // Sample bottom half of ROI, using same rate as ROI detector
        let sample_rate = (scale_value(SAMPLE_RATE_BASE, self.scale_factor) as usize).max(5);
        let half_h = rh / 2;
        let mut grey_samples: Vec<u8> = Vec::with_capacity(rw * (half_h / sample_rate + 1));

        for row_offset in (half_h..rh).step_by(sample_rate) {
            let y = ry + row_offset;
            for dx in 0..rw {
                let x = rx + dx;
                let idx = (y * iw + x) * 3;
                let r = image[idx];
                let g = image[idx + 1];
                let b = image[idx + 2];
                let chroma = r.max(g).max(b) - r.min(g).min(b);
                // Low chroma = grey (box), high chroma = colored (icon)
                if chroma <= 24 && r > 14 {
                    let grey = ((r as u16 + g as u16 + b as u16) / 3) as u8;
                    grey_samples.push(grey);
                }
            }
        }

        // Use (median - margin) as threshold to exclude icon edges
        let threshold = if grey_samples.len() >= 10 {
            grey_samples.sort_unstable();
            let median = grey_samples[grey_samples.len() / 2];
            median.saturating_sub(15).max(14)
        } else {
            14u8
        };

        let mask = self.create_threshold_mask_roi(image, iw, rx, ry, rw, rh, threshold);

        // Step 3: Find contours
        let contours = find_contours(&mask, rw, rh);

        // Step 4: Filter contours by size
        let valid_boxes: Vec<BoundingRect> = contours
            .into_iter()
            .filter(|&(_, _, cw, ch)| self.is_valid_box_size(cw, ch))
            .collect();

        if valid_boxes.is_empty() {
            return Err(FsOcrError::NoStockpileDetected);
        }

        // Step 5: Sort boxes (no adaptive threshold needed)
        let mut sorted_boxes = valid_boxes;
        sorted_boxes.sort_by(|a, b| {
            let y_cmp = a.1.cmp(&b.1);
            if y_cmp == std::cmp::Ordering::Equal {
                a.0.cmp(&b.0)
            } else {
                y_cmp
            }
        });

        // Step 6: Group boxes
        let (quantity_boxes, groups) = self.group_boxes(&sorted_boxes);

        if quantity_boxes.is_empty() {
            return Err(FsOcrError::NoStockpileDetected);
        }

        // Build result - coordinates are relative to ROI
        let mut regions =
            DetectedRegions::new(self.scale_factor, roi_h, self.box_width, self.box_height);
        regions.quantity_boxes = quantity_boxes;
        regions.groups = groups;
        regions.icon_regions = self.compute_icon_regions(&regions.quantity_boxes);

        Ok(regions)
    }

    /// Find the most common grey value in the ROI.
    ///
    /// Create a mask using a threshold - pixels with any RGB > threshold are white.
    #[allow(clippy::too_many_arguments)]
    fn create_threshold_mask_roi(
        &self,
        image: &[u8],
        img_width: usize,
        roi_x: usize,
        roi_y: usize,
        roi_w: usize,
        roi_h: usize,
        threshold: u8,
    ) -> Vec<u8> {
        // Max chroma for grey pixels (excludes colored icons)
        const MAX_CHROMA: u8 = 24;

        (0..roi_h)
            .into_par_iter()
            .flat_map(|dy| {
                let y = roi_y + dy;
                (0..roi_w)
                    .map(|dx| {
                        let x = roi_x + dx;
                        let idx = (y * img_width + x) * 3;
                        let r = image[idx];
                        let g = image[idx + 1];
                        let b = image[idx + 2];

                        // Check brightness threshold
                        let bright_enough = r > threshold && g > threshold && b > threshold;

                        // Check low chroma (grey, not colored)
                        let max_rgb = r.max(g).max(b);
                        let min_rgb = r.min(g).min(b);
                        let chroma = max_rgb - min_rgb;
                        let is_grey = chroma <= MAX_CHROMA;

                        if bright_enough && is_grey {
                            255u8
                        } else {
                            0u8
                        }
                    })
                    .collect::<Vec<u8>>()
            })
            .collect()
    }

    /// Internal detection with configurable adaptive threshold.
    fn detect_internal(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
        use_adaptive: bool,
    ) -> Result<DetectedRegions> {
        let w = width as usize;
        let h = height as usize;

        if image.len() != w * h * 3 {
            return Err(FsOcrError::Image(format!(
                "Invalid image size: expected {}x{}x3={}, got {}",
                w,
                h,
                w * h * 3,
                image.len()
            )));
        }

        // Step 1: Create grey mask with initial thresholds
        let mask = self.create_grey_mask(image, w, h);

        // Step 2: Apply morphological operations
        let processed = self.apply_morphology(&mask, w, h);

        // Step 3: Find contours (bounding boxes of connected components)
        let contours = find_contours(&processed, w, h);

        // Step 4: Filter contours by size (first pass)
        let valid_boxes: Vec<BoundingRect> = contours
            .into_iter()
            .filter(|&(_, _, cw, ch)| self.is_valid_box_size(cw, ch))
            .collect();

        // Step 5: Adaptive thresholding (only if enabled and enough boxes found)
        let final_boxes = if use_adaptive && valid_boxes.len() >= 2 {
            let (boxes, _) = self.apply_adaptive_threshold(image, w, h, &valid_boxes, &processed);
            boxes
        } else {
            valid_boxes
        };

        if final_boxes.is_empty() {
            return Err(FsOcrError::NoStockpileDetected);
        }

        // Step 6: Sort boxes by position (top-to-bottom, left-to-right)
        let mut sorted_boxes = final_boxes;
        sorted_boxes.sort_by(|a, b| {
            let y_cmp = a.1.cmp(&b.1);
            if y_cmp == std::cmp::Ordering::Equal {
                a.0.cmp(&b.0)
            } else {
                y_cmp
            }
        });

        // Step 7: Group boxes into rows and groups
        let (quantity_boxes, groups) = self.group_boxes(&sorted_boxes);

        if quantity_boxes.is_empty() {
            return Err(FsOcrError::NoStockpileDetected);
        }

        // Build result
        let mut regions =
            DetectedRegions::new(self.scale_factor, height, self.box_width, self.box_height);
        regions.quantity_boxes = quantity_boxes;
        regions.groups = groups;

        // Compute icon regions (offset from quantity boxes)
        regions.icon_regions = self.compute_icon_regions(&regions.quantity_boxes);

        // Compute stockpile type/name/shard regions
        self.detect_stockpile_regions(&mut regions);

        Ok(regions)
    }

    /// Apply adaptive thresholding based on measured grey values from detected boxes.
    ///
    /// Returns the final boxes and whether adaptive threshold was applied.
    fn apply_adaptive_threshold(
        &self,
        image: &[u8],
        w: usize,
        h: usize,
        valid_boxes: &[BoundingRect],
        processed_mask: &[u8],
    ) -> (Vec<BoundingRect>, bool) {
        // Measure grey values from detected boxes
        let mut background_greys: Vec<u8> = Vec::new();

        for &(x, y, bw, bh) in valid_boxes {
            let x = x as usize;
            let y = y as usize;
            let bw = bw as usize;
            let bh = bh as usize;

            // Collect grey values where mask is white
            for dy in 0..bh {
                for dx in 0..bw {
                    let py = y + dy;
                    let px = x + dx;
                    if py < h && px < w {
                        let mask_idx = py * w + px;
                        if processed_mask[mask_idx] > 0 {
                            // Get the grey value (average of RGB)
                            let img_idx = mask_idx * 3;
                            let r = image[img_idx] as u32;
                            let g = image[img_idx + 1] as u32;
                            let b = image[img_idx + 2] as u32;
                            let grey = ((r + g + b) / 3) as u8;
                            background_greys.push(grey);
                        }
                    }
                }
            }
        }

        if background_greys.is_empty() {
            return (valid_boxes.to_vec(), false);
        }

        // Calculate median
        background_greys.sort_unstable();
        let median = background_greys[background_greys.len() / 2];

        // Calculate adaptive range: median ± 5 (bounded by original limits)
        const MARGIN: u8 = 5;
        let adaptive_lower = median.saturating_sub(MARGIN).max(self.gray_lower);
        let adaptive_upper = median.saturating_add(MARGIN).min(self.gray_upper);

        // Only re-detect if adaptive range is tighter
        if adaptive_lower <= self.gray_lower && adaptive_upper >= self.gray_upper {
            return (valid_boxes.to_vec(), false);
        }

        // Second pass with adaptive thresholds
        let adaptive_mask =
            self.create_grey_mask_with_range(image, w, h, adaptive_lower, adaptive_upper);
        let adaptive_processed = self.apply_morphology(&adaptive_mask, w, h);
        let adaptive_contours = find_contours(&adaptive_processed, w, h);

        let adaptive_boxes: Vec<BoundingRect> = adaptive_contours
            .into_iter()
            .filter(|&(_, _, cw, ch)| self.is_valid_box_size(cw, ch))
            .collect();

        // Return adaptive result if it found boxes, otherwise fall back to original
        if adaptive_boxes.is_empty() {
            (valid_boxes.to_vec(), false)
        } else {
            (adaptive_boxes, true)
        }
    }

    /// Detect stockpile type, name, and shard regions.
    fn detect_stockpile_regions(&self, regions: &mut DetectedRegions) {
        if regions.quantity_boxes.is_empty() {
            return;
        }

        let (first_x, first_y) = regions.quantity_boxes[0];

        // Find max x coordinate across all quantity boxes
        let max_detected_x = regions
            .quantity_boxes
            .iter()
            .map(|&(x, _)| x)
            .max()
            .unwrap_or(first_x);

        // Title region calculations
        let title_min_x = first_x - self.column_offset + self.box_width;
        let title_y = first_y - self.row_offset;

        let title_max_x = (max_detected_x + self.box_width + self.title_margin)
            .max(title_min_x + self.title_min_width);

        // Stockpile type region (blue rectangle in Python)
        regions.type_region = Some((
            title_min_x + self.title_margin * 3 / 4,
            title_y + self.title_height / 8,
            self.stockpile_type_width,
            self.title_height * 3 / 4,
        ));

        // Stockpile name region (green rectangle in Python)
        let name_x = title_max_x - self.stockpile_name_width - self.box_width;
        regions.name_region = Some((
            name_x,
            title_y,
            self.stockpile_name_width,
            self.title_height,
        ));

        // Shard/timestamp region (bottom-left corner)
        let shard_x = self.box_height;
        let shard_y = self.image_height - self.box_height * 3;
        regions.shard_region = Some((shard_x, shard_y, self.shard_width, self.box_height));
    }

    /// Detect stockpile type, name, and shard regions based on info bar height.
    ///
    /// Info bar heights at 2160p:
    /// - Old format: GROUP_OFFSET - ROW_OFFSET - GREY_BAR_HEIGHT = 14px
    /// - No custom name: ROW_OFFSET = 78px
    /// - Pinned: COLUMN_OFFSET = 112px
    /// - Unpinned: ROW_OFFSET + BOX_HEIGHT + GREY_BAR_HEIGHT = 148px
    pub fn detect_stockpile_regions_with_info_bar(
        &self,
        regions: &mut DetectedRegions,
        roi_x: i32,
        roi_y: i32,
    ) {
        if regions.quantity_boxes.is_empty() {
            return;
        }

        let (first_x, first_y) = regions.quantity_boxes[0];
        let info_bar_height = regions.info_bar_height;

        // Find max x coordinate across all quantity boxes
        let max_detected_x = regions
            .quantity_boxes
            .iter()
            .map(|&(x, _)| x)
            .max()
            .unwrap_or(first_x);

        // Icon x is offset to the left of quantity box
        let icon_to_qty_offset = scale_value(ICON_TO_QUANTITY_OFFSET, self.scale_factor);
        let first_icon_x = first_x - icon_to_qty_offset;

        // Title region calculations
        let title_min_x = first_icon_x - self.title_margin / 2;
        let title_max_x = (max_detected_x + self.box_width + self.title_margin)
            .max(title_min_x + self.title_min_width);

        // Stockpile type region: a box-height band sits just above the ROI, but
        // the type label only fills a slab of it (the rest is a noise strip above
        // and grey background below). Crop directly to that measured slab —
        // [`STOCKPILE_TYPE_TOP_FACTOR`]..+[`STOCKPILE_TYPE_HEIGHT_FACTOR`] of the
        // band — so the region is text-only without any runtime band-finding.
        // Width is 4x box_width (stockpile_type_width); horizontal trim handles
        // the variable word length.
        let band_top = roi_y - self.box_height;
        let type_y = band_top + (self.box_height as f64 * STOCKPILE_TYPE_TOP_FACTOR).round() as i32;
        let type_h =
            ((self.box_height as f64 * STOCKPILE_TYPE_HEIGHT_FACTOR).round() as i32).max(1);
        regions.type_region = Some((
            roi_x + self.box_width / 4,
            type_y,
            self.stockpile_type_width,
            type_h,
        ));

        // Shard/timestamp region (bottom-left corner)
        let shard_x = self.box_height;
        let shard_y = self.image_height - self.box_height * 3;
        regions.shard_region = Some((shard_x, shard_y, self.shard_width, self.box_height));

        // Thresholds at 2160p (derived from constants):
        // - Old format: < (GROUP_OFFSET - ROW_OFFSET) = 20
        // - No name: < GROUP_OFFSET = 98
        // - Pinned: < ROW_OFFSET + BOX_HEIGHT = 142
        // - Unpinned: >= 142
        let threshold_old = self.group_offset - self.row_offset;
        let threshold_no_name = self.group_offset;
        let threshold_pinned = self.row_offset + self.box_height;

        if info_bar_height < threshold_old {
            // Old format: name sits on the same line as the type, so it shares the
            // type region's text slab (same Y and height) — not the full box-height
            // band, which would re-introduce the noise strip above / grey below.
            let name_x = title_max_x - self.stockpile_name_width - self.box_width;
            regions.name_region = Some((name_x, type_y, self.stockpile_name_width, type_h));
        } else if info_bar_height < threshold_no_name {
            // No custom name
            regions.name_region = None;
        } else if info_bar_height < threshold_pinned {
            // Pinned: name at same X as type region
            let type_margin = self.box_width / 4;
            let name_x = roi_x + type_margin;
            let top_margin = scale_value(3, self.scale_factor);
            let name_y = first_y - self.box_height - self.title_margin / 6 + top_margin;
            let name_w = self.box_width + self.title_height - self.title_margin / 12;
            let name_h = self.title_height / 2 + self.title_margin / 12 - top_margin;
            regions.name_region = Some((name_x, name_y, name_w, name_h));
        } else {
            // Unpinned: custom name in info bar area
            let name_x = first_x - self.title_margin / 2;
            let name_y = first_y - self.group_offset;
            let name_w = self.box_width * 2;
            let name_h = self.title_height;
            regions.name_region = Some((name_x, name_y, name_w, name_h));
        }
    }

    /// Create a grey mask from an RGB image (parallelized).
    ///
    /// Uses dual-mask approach: HSV saturation check + RGB value check.
    fn create_grey_mask(&self, image: &[u8], width: usize, height: usize) -> Vec<u8> {
        self.create_grey_mask_with_range(image, width, height, self.gray_lower, self.gray_upper)
    }

    /// Create a grey mask with custom grey thresholds.
    fn create_grey_mask_with_range(
        &self,
        image: &[u8],
        width: usize,
        height: usize,
        gray_lower: u8,
        gray_upper: u8,
    ) -> Vec<u8> {
        (0..height)
            .into_par_iter()
            .flat_map(|y| {
                (0..width)
                    .map(|x| {
                        let idx = (y * width + x) * 3;
                        let r = image[idx];
                        let g = image[idx + 1];
                        let b = image[idx + 2];

                        if Self::is_grey_pixel_static(r, g, b, gray_lower, gray_upper) {
                            255u8
                        } else {
                            0u8
                        }
                    })
                    .collect::<Vec<u8>>()
            })
            .collect()
    }

    /// Static version of is_grey_pixel for use in parallel iterators.
    #[inline]
    fn is_grey_pixel_static(r: u8, g: u8, b: u8, gray_lower: u8, gray_upper: u8) -> bool {
        // Method 1: RGB balance check (all channels similar)
        if r < gray_lower
            || r > gray_upper
            || g < gray_lower
            || g > gray_upper
            || b < gray_lower
            || b > gray_upper
        {
            return false;
        }

        // Method 2: HSV saturation check (low saturation = grey)
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);

        if max == 0 {
            return true; // Black is considered grey
        }

        // Saturation = (max - min) / max * 255
        let saturation = ((max - min) as u32 * 255) / max as u32;

        // Low saturation means grey (< 30 out of 255)
        saturation < 30
    }

    /// Apply morphological close (fill gaps) and open (separate merged boxes).
    fn apply_morphology(&self, mask: &[u8], width: usize, height: usize) -> Vec<u8> {
        // Close operation (dilate then erode) - fills small gaps
        let closed = dilate(mask, width, height, 3);
        let closed = erode(&closed, width, height, 3);

        // Open operation (erode then dilate) - separates merged boxes
        let opened = erode(&closed, width, height, 5);
        dilate(&opened, width, height, 5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detector_scaling() {
        let detector = GreyMaskDetector::new(3840, 2160);
        assert!((detector.scale_factor - 1.0).abs() < 0.001);
        assert_eq!(detector.box_width, 84);
        assert_eq!(detector.box_height, 64);
    }

    #[test]
    fn test_detector_scaling_1080p() {
        let detector = GreyMaskDetector::new(1920, 1080);
        assert!((detector.scale_factor - 0.5).abs() < 0.001);
        assert_eq!(detector.box_width, 42);
        assert_eq!(detector.box_height, 32);
    }

    #[test]
    fn test_is_grey_pixel() {
        use crate::constants::{GRAY_LOWER, GRAY_UPPER};

        // Pure grey should match
        assert!(GreyMaskDetector::is_grey_pixel_static(
            50, 50, 50, GRAY_LOWER, GRAY_UPPER
        ));

        // Near-grey should match
        assert!(GreyMaskDetector::is_grey_pixel_static(
            48, 50, 52, GRAY_LOWER, GRAY_UPPER
        ));

        // Colored pixel should not match
        assert!(!GreyMaskDetector::is_grey_pixel_static(
            255, 0, 0, GRAY_LOWER, GRAY_UPPER
        ));

        // Too dark should not match
        assert!(!GreyMaskDetector::is_grey_pixel_static(
            5, 5, 5, GRAY_LOWER, GRAY_UPPER
        ));

        // Too bright should not match
        assert!(!GreyMaskDetector::is_grey_pixel_static(
            200, 200, 200, GRAY_LOWER, GRAY_UPPER
        ));
    }

    #[test]
    fn test_detect_no_boxes() {
        let detector = GreyMaskDetector::new(100, 100);
        let image = vec![0u8; 100 * 100 * 3]; // All black

        let result = detector.detect(&image, 100, 100);
        assert!(result.is_err());
    }
}

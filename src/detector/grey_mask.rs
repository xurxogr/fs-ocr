//! Grey mask detection for stockpile quantity boxes.
//!
//! Uses a dual-mask approach combining HSV and RGB color space
//! for robust grey detection across different lighting conditions.

use rayon::prelude::*;

use crate::constants::{
    compute_scale_factor, scale_value, BOX_HEIGHT, BOX_WIDTH, COLUMN_OFFSET, GRAY_LOWER,
    GRAY_UPPER, GROUP_OFFSET, ICON_TO_QUANTITY_OFFSET, PIXEL_DIFF_TOLERANCE, ROW_OFFSET,
    SHARD_WIDTH_FACTOR, STOCKPILE_NAME_WIDTH_FACTOR, STOCKPILE_TYPE_WIDTH_FACTOR, TITLE_HEIGHT,
    TITLE_MARGIN, TITLE_MIN_WIDTH,
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

    /// Detect within an ROI region.
    #[allow(clippy::too_many_arguments)]
    pub fn detect_roi(
        &self,
        image: &[u8],
        img_width: i32,
        img_height: i32,
        roi_x: i32,
        roi_y: i32,
        roi_w: i32,
        roi_h: i32,
    ) -> Result<DetectedRegions> {
        let iw = img_width as usize;
        let ih = img_height as usize;
        let rx = roi_x as usize;
        let ry = roi_y as usize;
        let rw = roi_w as usize;
        let rh = roi_h as usize;

        if image.len() != iw * ih * 3 {
            return Err(FsOcrError::Image(format!(
                "Invalid image size: expected {}x{}x3={}, got {}",
                iw,
                ih,
                iw * ih * 3,
                image.len()
            )));
        }

        // Step 1: Create grey mask for ROI region
        let mask = self.create_grey_mask_roi(image, iw, rx, ry, rw, rh);

        // Step 2: Find contours directly (skip morphology - ROI is clean)
        let contours = find_contours(&mask, rw, rh);

        // Step 4: Filter contours by size
        let valid_boxes: Vec<BoundingRect> = contours
            .into_iter()
            .filter(|&(_, _, cw, ch)| self.is_valid_box_size(cw, ch))
            .collect();

        if valid_boxes.is_empty() {
            return Err(FsOcrError::NoStockpileDetected);
        }

        // Skip adaptive threshold - ROI is already validated by black box detection
        let final_boxes = valid_boxes;

        // Step 6: Sort boxes
        let mut sorted_boxes = final_boxes;
        sorted_boxes.sort_by(|a, b| {
            let y_cmp = a.1.cmp(&b.1);
            if y_cmp == std::cmp::Ordering::Equal {
                a.0.cmp(&b.0)
            } else {
                y_cmp
            }
        });

        // Step 7: Group boxes
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

        // Create mask using fixed threshold of 14
        // Works for all image types:
        // - Normal: black=0-5, boxes=76-80 → 76 > 14 ✓
        // - Dark: black=0-1, boxes=79-80 → 79 > 14 ✓
        // - Gamma dark: black=0, boxes=15 → 15 > 14 ✓
        // - Light: black=0-2, boxes=166-167 → 166 > 14 ✓
        const BOX_THRESHOLD: u8 = 14;
        let mask = self.create_threshold_mask_roi(image, iw, rx, ry, rw, rh, BOX_THRESHOLD);

        // Step 3: Find contours directly (no morphology - higher threshold keeps boxes separated)
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
            DetectedRegions::new(self.scale_factor, roi_h as i32, self.box_width, self.box_height);
        regions.quantity_boxes = quantity_boxes;
        regions.groups = groups;
        regions.icon_regions = self.compute_icon_regions(&regions.quantity_boxes);

        Ok(regions)
    }

    /// Find the most common grey value in the ROI.
    ///
    /// Create a mask using a threshold - pixels with any RGB > threshold are white.
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

                        // All channels must exceed threshold to exclude noise pixels
                        if r > threshold && g > threshold && b > threshold {
                            255u8
                        } else {
                            0u8
                        }
                    })
                    .collect::<Vec<u8>>()
            })
            .collect()
    }

    /// Create a grey mask for an ROI region within a larger image.
    fn create_grey_mask_roi(
        &self,
        image: &[u8],
        img_width: usize,
        roi_x: usize,
        roi_y: usize,
        roi_w: usize,
        roi_h: usize,
    ) -> Vec<u8> {
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

                        if Self::is_grey_pixel_static(r, g, b, self.gray_lower, self.gray_upper) {
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
        let title_y = roi_y;

        let title_max_x = (max_detected_x + self.box_width + self.title_margin)
            .max(title_min_x + self.title_min_width);

        // Stockpile type region: ABOVE the ROI, narrow width to avoid background
        // Width is 4x box_width (stockpile_type_width), height is box_height
        regions.type_region = Some((
            roi_x + self.box_width / 4,
            roi_y - self.box_height,
            self.stockpile_type_width,
            self.box_height,
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
            // Old format: name at same Y level as type region (above ROI)
            let name_x = title_max_x - self.stockpile_name_width - self.box_width;
            let name_y = roi_y - self.box_height; // Same Y as type_region
            regions.name_region = Some((
                name_x,
                name_y,
                self.stockpile_name_width,
                self.box_height, // Same height as type_region
            ));
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

    /// Check if dimensions match expected box size.
    fn is_valid_box_size(&self, width: i32, height: i32) -> bool {
        self.in_valid_range(width, self.box_width) && self.in_valid_range(height, self.box_height)
    }

    /// Check if two values are within tolerance.
    fn in_valid_range(&self, first: i32, second: i32) -> bool {
        (first - second).abs() <= PIXEL_DIFF_TOLERANCE
    }

    /// Group detected boxes into rows and groups with grid validation.
    ///
    /// This implements Python's approach:
    /// 1. Find the first valid box pair to establish the grid
    /// 2. Compute expected column positions (6 columns)
    /// 3. Only include boxes that match expected column positions
    fn group_boxes(&self, boxes: &[BoundingRect]) -> (Vec<Coordinates>, Vec<GroupInfo>) {
        if boxes.is_empty() {
            return (Vec::new(), Vec::new());
        }

        // Step 1: Detect first group and establish grid
        let (first_group, start_idx, valid_x_positions) = match self.detect_first_group(boxes) {
            Some(result) => result,
            None => return (Vec::new(), Vec::new()),
        };

        let mut quantity_boxes: Vec<Coordinates> = first_group.clone();
        let mut groups: Vec<GroupInfo> = Vec::new();

        let first_group_size = first_group.len();
        let mut current_group_count = first_group_size;
        let mut current_group_start_idx = 0;
        let mut last_y = first_group[0].1;
        let mut current_x_idx = first_group_size;
        let mut current_group_idx = 0;

        // Step 2: Process remaining boxes with grid validation
        for &(x, y, _, _) in &boxes[start_idx..] {
            // Determine expected column index
            let expected_column_index = current_x_idx % 6;

            // Validate against expected x position
            let matches_expected = self.in_valid_range(x, valid_x_positions[expected_column_index]);
            let matches_first_col = self.in_valid_range(x, valid_x_positions[0]);

            if !matches_expected && !matches_first_col {
                // Box doesn't match expected grid - skip
                continue;
            }

            // Reset to column 0 if box matches first column but not expected
            if !matches_expected && matches_first_col {
                current_x_idx = 0;
            }

            let y_diff = (y - last_y).abs();

            // Check if this is a new group
            let is_new_group = self.in_valid_range(y_diff, self.group_offset)
                || (current_group_idx == 0
                    && (self.in_valid_range(y_diff, self.group_offset * 2)
                        || self.in_valid_range(y_diff, self.group_offset * 2 + self.row_offset)));
            let is_same_row = self.in_valid_range(y_diff, 0);
            let is_next_row = self.in_valid_range(y_diff, self.row_offset);

            if is_new_group {
                // Save current group and start new one
                if current_group_count > 0 {
                    groups.push(GroupInfo::new(current_group_count, current_group_start_idx));
                }
                current_group_start_idx = quantity_boxes.len();
                current_group_count = 0;
                current_x_idx = 0;
                current_group_idx += 1;
            } else if !is_same_row && !is_next_row {
                // Invalid row - skip
                continue;
            }

            quantity_boxes.push((x, y));
            current_group_count += 1;
            last_y = y;
            current_x_idx += 1;
        }

        // Save final group
        if current_group_count > 0 {
            groups.push(GroupInfo::new(current_group_count, current_group_start_idx));
        }

        (quantity_boxes, groups)
    }

    /// Detect the first group (1 or 2 boxes) and establish the grid.
    ///
    /// Returns: (first_group_boxes, next_index_to_process, valid_x_positions)
    fn detect_first_group(
        &self,
        boxes: &[BoundingRect],
    ) -> Option<(Vec<Coordinates>, usize, Vec<i32>)> {
        let mut first_box_idx = 0;

        while first_box_idx < boxes.len().saturating_sub(1) {
            let (x1, y1, _, _) = boxes[first_box_idx];

            // Look for second box in same row
            let mut second_box_idx = first_box_idx + 1;
            while second_box_idx < boxes.len() {
                let (x2, y2, _, _) = boxes[second_box_idx];
                let x_diff = (x2 - x1).abs();

                // Check if in same row
                if self.in_valid_range(y1, y2) {
                    if self.in_valid_range(x_diff, self.column_offset) {
                        // Found 2-box pair - establish grid
                        let first_column_x = x1.min(x2);
                        let valid_x_positions = self.compute_valid_x_positions(first_column_x);
                        return Some((
                            vec![(x1, y1), (x2, y2)],
                            second_box_idx + 1,
                            valid_x_positions,
                        ));
                    }
                    second_box_idx += 1;
                    continue;
                }

                // Second box is in different row - check if same column (single-item first row)
                if self.in_valid_range(x1, x2) {
                    let valid_x_positions = self.compute_valid_x_positions(x1);
                    return Some((vec![(x1, y1)], first_box_idx + 1, valid_x_positions));
                }

                // Different column in different row - skip and keep looking
                second_box_idx += 1;
            }

            first_box_idx += 1;
        }

        // Last resort: use single box if at least one exists
        if !boxes.is_empty() {
            let (x, y, _, _) = boxes[0];
            let valid_x_positions = self.compute_valid_x_positions(x);
            return Some((vec![(x, y)], 1, valid_x_positions));
        }

        None
    }

    /// Compute valid x positions for 6 columns based on first column x.
    ///
    /// Uses floating-point calculation for each offset to avoid cumulative rounding errors.
    /// Matches Python's approach: round(column_offset_float * i) for each column.
    fn compute_valid_x_positions(&self, first_column_x: i32) -> Vec<i32> {
        // Use unscaled column offset for precise calculation
        let column_offset_float = (COLUMN_OFFSET + BOX_WIDTH) as f64 * self.scale_factor;

        (0..6)
            .map(|i| first_column_x + (column_offset_float * i as f64).round() as i32)
            .collect()
    }

    /// Debug: Get all contours found before size filtering.
    pub fn debug_get_all_contours(
        &self,
        image: &[u8],
        width: i32,
        height: i32,
    ) -> Result<Vec<BoundingRect>> {
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

        let mask = self.create_grey_mask(image, w, h);
        let processed = self.apply_morphology(&mask, w, h);
        let contours = find_contours(&processed, w, h);

        Ok(contours)
    }

    /// Compute icon regions from quantity box positions.
    fn compute_icon_regions(&self, quantity_boxes: &[Coordinates]) -> Vec<BoundingRect> {
        let icon_offset = scale_value(88, self.scale_factor); // ICON_TO_QUANTITY_OFFSET

        quantity_boxes
            .iter()
            .map(|&(x, y)| {
                // Icon is to the left of the quantity box
                // Icons are square using box_height for both dimensions (64x64 at 2160p)
                // X: starts at x - offset, width is box_height
                let icon_x = x - icon_offset;
                (icon_x, y, self.box_height, self.box_height)
            })
            .collect()
    }
}

/// Parallel dilation operation using separable 1D passes.
/// Much faster than naive 2D kernel approach.
fn dilate(image: &[u8], width: usize, height: usize, kernel_size: usize) -> Vec<u8> {
    let half = kernel_size / 2;

    // Horizontal pass
    let horizontal: Vec<u8> = (0..height)
        .into_par_iter()
        .flat_map(|y| {
            let row_start = y * width;
            (0..width)
                .map(|x| {
                    let start = x.saturating_sub(half);
                    let end = (x + half + 1).min(width);
                    image[row_start + start..row_start + end]
                        .iter()
                        .copied()
                        .max()
                        .unwrap_or(0)
                })
                .collect::<Vec<u8>>()
        })
        .collect();

    // Vertical pass
    (0..height)
        .into_par_iter()
        .flat_map(|y| {
            let y_start = y.saturating_sub(half);
            let y_end = (y + half + 1).min(height);
            (0..width)
                .map(|x| {
                    (y_start..y_end)
                        .map(|ny| horizontal[ny * width + x])
                        .max()
                        .unwrap_or(0)
                })
                .collect::<Vec<u8>>()
        })
        .collect()
}

/// Parallel erosion operation using separable 1D passes.
fn erode(image: &[u8], width: usize, height: usize, kernel_size: usize) -> Vec<u8> {
    let half = kernel_size / 2;

    // Horizontal pass
    let horizontal: Vec<u8> = (0..height)
        .into_par_iter()
        .flat_map(|y| {
            let row_start = y * width;
            (0..width)
                .map(|x| {
                    // Handle boundary: if kernel goes out of bounds, result is 0
                    if x < half || x + half >= width {
                        // Check if any part would be out of bounds
                        let start = x.saturating_sub(half);
                        let end = (x + half + 1).min(width);
                        if end - start < kernel_size {
                            return 0; // Out of bounds
                        }
                    }
                    let start = x.saturating_sub(half);
                    let end = (x + half + 1).min(width);
                    image[row_start + start..row_start + end]
                        .iter()
                        .copied()
                        .min()
                        .unwrap_or(0)
                })
                .collect::<Vec<u8>>()
        })
        .collect();

    // Vertical pass
    (0..height)
        .into_par_iter()
        .flat_map(|y| {
            (0..width)
                .map(|x| {
                    // Handle boundary
                    if y < half || y + half >= height {
                        return 0;
                    }
                    let y_start = y.saturating_sub(half);
                    let y_end = (y + half + 1).min(height);
                    (y_start..y_end)
                        .map(|ny| horizontal[ny * width + x])
                        .min()
                        .unwrap_or(0)
                })
                .collect::<Vec<u8>>()
        })
        .collect()
}

/// Find connected components and return their bounding boxes.
fn find_contours(mask: &[u8], width: usize, height: usize) -> Vec<BoundingRect> {
    // Simple connected component labeling
    let mut labels = vec![0u32; width * height];
    let mut current_label = 1u32;
    let mut equivalences: Vec<u32> = vec![0]; // equivalences[label] = root label

    // First pass: label pixels and track equivalences
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if mask[idx] == 0 {
                continue;
            }

            let mut neighbors = Vec::new();

            // Check left neighbor
            if x > 0 && labels[idx - 1] > 0 {
                neighbors.push(labels[idx - 1]);
            }

            // Check top neighbor
            if y > 0 && labels[idx - width] > 0 {
                neighbors.push(labels[idx - width]);
            }

            if neighbors.is_empty() {
                // New label
                labels[idx] = current_label;
                equivalences.push(current_label);
                current_label += 1;
            } else {
                // Use minimum neighbor label
                let min_label = *neighbors.iter().min().unwrap();
                labels[idx] = min_label;

                // Record equivalences
                for &n in &neighbors {
                    if n != min_label {
                        union_find(&mut equivalences, min_label, n);
                    }
                }
            }
        }
    }

    // Second pass: resolve equivalences
    for label in labels.iter_mut() {
        if *label > 0 {
            *label = find_root(&equivalences, *label);
        }
    }

    // Find bounding boxes for each label
    let mut bounds: std::collections::HashMap<u32, (i32, i32, i32, i32)> =
        std::collections::HashMap::new();

    for y in 0..height {
        for x in 0..width {
            let label = labels[y * width + x];
            if label > 0 {
                let entry = bounds
                    .entry(label)
                    .or_insert((x as i32, y as i32, x as i32, y as i32));
                entry.0 = entry.0.min(x as i32);
                entry.1 = entry.1.min(y as i32);
                entry.2 = entry.2.max(x as i32);
                entry.3 = entry.3.max(y as i32);
            }
        }
    }

    // Convert to bounding rects
    bounds
        .values()
        .map(|&(min_x, min_y, max_x, max_y)| (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
        .collect()
}

/// Union-Find: merge two labels.
fn union_find(equivalences: &mut [u32], a: u32, b: u32) {
    let root_a = find_root(equivalences, a);
    let root_b = find_root(equivalences, b);
    if root_a != root_b {
        let min_root = root_a.min(root_b);
        let max_root = root_a.max(root_b);
        equivalences[max_root as usize] = min_root;
    }
}

/// Union-Find: find root label.
fn find_root(equivalences: &[u32], mut label: u32) -> u32 {
    while equivalences[label as usize] != label {
        label = equivalences[label as usize];
    }
    label
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
    fn test_valid_box_size() {
        let detector = GreyMaskDetector::new(3840, 2160);
        assert!(detector.is_valid_box_size(84, 64));
        assert!(detector.is_valid_box_size(85, 63)); // Within tolerance
        assert!(!detector.is_valid_box_size(90, 64)); // Outside tolerance
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
    fn test_dilate_erode() {
        // Create a simple test image with a white dot
        let mut mask = vec![0u8; 10 * 10];
        mask[4 * 10 + 4] = 255; // Center pixel

        // Dilation should expand the dot
        let dilated = dilate(&mask, 10, 10, 3);
        assert_eq!(dilated[4 * 10 + 4], 255);
        assert_eq!(dilated[4 * 10 + 5], 255); // Right neighbor
        assert_eq!(dilated[5 * 10 + 4], 255); // Bottom neighbor

        // Erosion of dilated should not be empty
        let eroded = erode(&dilated, 10, 10, 3);
        // Center region should still be white
        assert!(eroded.iter().filter(|&&x| x > 0).count() > 0);
    }

    #[test]
    fn test_find_contours_single_box() {
        // Create a 100x100 image with a single 20x20 white box
        let mut mask = vec![0u8; 100 * 100];
        for y in 40..60 {
            for x in 40..60 {
                mask[y * 100 + x] = 255;
            }
        }

        let contours = find_contours(&mask, 100, 100);
        assert_eq!(contours.len(), 1);

        let (x, y, w, h) = contours[0];
        assert_eq!(x, 40);
        assert_eq!(y, 40);
        assert_eq!(w, 20);
        assert_eq!(h, 20);
    }

    #[test]
    fn test_find_contours_multiple_boxes() {
        // Create image with two separate boxes
        let mut mask = vec![0u8; 100 * 100];

        // Box 1: top-left
        for y in 10..20 {
            for x in 10..20 {
                mask[y * 100 + x] = 255;
            }
        }

        // Box 2: bottom-right
        for y in 70..80 {
            for x in 70..80 {
                mask[y * 100 + x] = 255;
            }
        }

        let contours = find_contours(&mask, 100, 100);
        assert_eq!(contours.len(), 2);
    }

    #[test]
    fn test_detect_no_boxes() {
        let detector = GreyMaskDetector::new(100, 100);
        let image = vec![0u8; 100 * 100 * 3]; // All black

        let result = detector.detect(&image, 100, 100);
        assert!(result.is_err());
    }
}

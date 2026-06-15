//! Grouping detected boxes into the stockpile grid: row/column validation, first-group seeding, valid X positions, and icon-region geometry.

use crate::constants::{scale_value, BOX_WIDTH, COLUMN_OFFSET, PIXEL_DIFF_TOLERANCE};

use super::{BoundingRect, Coordinates, GreyMaskDetector, GroupInfo};

impl GreyMaskDetector {
    /// Check if dimensions match expected box size.
    pub(super) fn is_valid_box_size(&self, width: i32, height: i32) -> bool {
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
    pub(super) fn group_boxes(&self, boxes: &[BoundingRect]) -> (Vec<Coordinates>, Vec<GroupInfo>) {
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

    /// Compute icon regions from quantity box positions.
    pub(super) fn compute_icon_regions(&self, quantity_boxes: &[Coordinates]) -> Vec<BoundingRect> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_box_size() {
        let detector = GreyMaskDetector::new(3840, 2160);
        assert!(detector.is_valid_box_size(84, 64));
        assert!(detector.is_valid_box_size(85, 63)); // Within tolerance
        assert!(!detector.is_valid_box_size(90, 64)); // Outside tolerance
    }
}

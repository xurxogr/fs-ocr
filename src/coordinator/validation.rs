//! Validation utilities for OCR results.

use crate::detector::GroupInfo;
use crate::ocr::quantity::{is_descending, is_descending_across_rows};

/// Validate that quantities follow descending order within and across groups.
///
/// Args:
///     quantities: Flat list of all detected quantities.
///     groups: Group information for organizing quantities.
///     skip_first_group: Whether to skip validation for the first group.
///
/// Returns:
///     List of (index, error_message) for invalid quantities.
pub fn validate_descending_order(
    quantities: &[i32],
    groups: &[GroupInfo],
    skip_first_group: bool,
) -> Vec<(usize, String)> {
    let mut errors = Vec::new();

    for (group_idx, group) in groups.iter().enumerate() {
        // Skip first group if requested (base items can have any order)
        if skip_first_group && group_idx == 0 {
            continue;
        }

        let start = group.start_index;
        let end = start + group.size;

        if end > quantities.len() {
            errors.push((
                start,
                format!(
                    "Group {} extends beyond quantities (start={}, size={}, total={})",
                    group_idx,
                    start,
                    group.size,
                    quantities.len()
                ),
            ));
            continue;
        }

        let group_quantities = &quantities[start..end];

        // Check descending order within group
        if !is_descending(group_quantities) {
            for i in 1..group_quantities.len() {
                if group_quantities[i] > group_quantities[i - 1] {
                    errors.push((
                        start + i,
                        format!(
                            "Non-descending: {} > {} at index {}",
                            group_quantities[i],
                            group_quantities[i - 1],
                            start + i
                        ),
                    ));
                }
            }
        }

        // Check continuity with previous group (if not first)
        if group_idx > 0 && !skip_first_group {
            let prev_group = &groups[group_idx - 1];
            let prev_end = prev_group.start_index + prev_group.size;

            if prev_end <= quantities.len() && !group_quantities.is_empty() {
                let prev_last = quantities[prev_end - 1];
                let curr_first = group_quantities[0];

                if !is_descending_across_rows(prev_last, curr_first) {
                    errors.push((
                        start,
                        format!(
                            "Non-descending across groups: {} (prev last) < {} (curr first)",
                            prev_last, curr_first
                        ),
                    ));
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_descending_valid() {
        let quantities = vec![100, 50, 25, 10, 5];
        let groups = vec![GroupInfo::new(2, 0), GroupInfo::new(3, 2)];

        let errors = validate_descending_order(&quantities, &groups, false);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validate_descending_invalid() {
        let quantities = vec![100, 50, 75, 10, 5]; // 75 > 50 is invalid
        let groups = vec![GroupInfo::new(2, 0), GroupInfo::new(3, 2)];

        let errors = validate_descending_order(&quantities, &groups, false);
        assert!(!errors.is_empty());
    }

    #[test]
    fn test_validate_descending_skip_first_group() {
        let quantities = vec![50, 100, 25, 10, 5]; // First group can have any order
        let groups = vec![GroupInfo::new(2, 0), GroupInfo::new(3, 2)];

        let errors = validate_descending_order(&quantities, &groups, true);
        assert!(errors.is_empty());
    }
}

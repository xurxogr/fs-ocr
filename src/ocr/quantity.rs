//! Quantity parsing from OCR text.

/// Validate that quantities are in descending order within a row.
pub fn is_descending(quantities: &[i32]) -> bool {
    quantities.windows(2).all(|w| w[0] >= w[1])
}

/// Validate descending order across consecutive rows in a group.
pub fn is_descending_across_rows(prev_last: i32, curr_first: i32) -> bool {
    prev_last >= curr_first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_descending() {
        assert!(is_descending(&[100, 50, 25]));
        assert!(is_descending(&[100, 100, 50])); // Equal is ok
        assert!(!is_descending(&[100, 150, 50]));
        assert!(is_descending(&[]));
        assert!(is_descending(&[100]));
    }

    #[test]
    fn test_is_descending_across_rows() {
        assert!(is_descending_across_rows(50, 25));
        assert!(is_descending_across_rows(50, 50)); // Equal is ok
        assert!(!is_descending_across_rows(25, 50));
    }
}

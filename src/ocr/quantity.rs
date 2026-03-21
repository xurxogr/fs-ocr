//! Quantity parsing from OCR text.

/// Parse a single quantity value from text.
///
/// Handles:
/// - Plain numbers: "150" -> 150
/// - Thousands suffix: "500k+" -> 500000
/// - Thousands suffix without plus: "100k" -> 100000
pub fn parse_quantity(text: &str) -> Option<i32> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Handle "k+" or "k" suffix
    if let Some(base) = trimmed
        .strip_suffix("k+")
        .or_else(|| trimmed.strip_suffix("k"))
    {
        base.parse::<i32>().ok().map(|n| n * 1000)
    } else if let Some(base) = trimmed.strip_suffix('+') {
        // Just "+" without "k" doesn't multiply
        base.parse::<i32>().ok()
    } else {
        trimmed.parse::<i32>().ok()
    }
}

/// Parse multiple quantity lines from OCR output.
///
/// Input format:
/// ```text
/// 150 100 50
/// 25 10
/// ```
///
/// Returns nested vectors representing rows and columns.
pub fn parse_quantity_text(text: &str) -> Vec<Vec<i32>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split_whitespace().filter_map(parse_quantity).collect())
        .filter(|row: &Vec<i32>| !row.is_empty())
        .collect()
}

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
    fn test_parse_quantity() {
        assert_eq!(parse_quantity("150"), Some(150));
        assert_eq!(parse_quantity("500k+"), Some(500000));
        assert_eq!(parse_quantity("100k"), Some(100000));
        assert_eq!(parse_quantity("999+"), Some(999));
        assert_eq!(parse_quantity(""), None);
        assert_eq!(parse_quantity("abc"), None);
    }

    #[test]
    fn test_parse_quantity_text() {
        let text = "150 100 50\n25 10\n";
        let result = parse_quantity_text(text);
        assert_eq!(result, vec![vec![150, 100, 50], vec![25, 10]]);
    }

    #[test]
    fn test_parse_quantity_text_with_k() {
        let text = "500k+ 100k 50k\n25 10\n";
        let result = parse_quantity_text(text);
        assert_eq!(result, vec![vec![500000, 100000, 50000], vec![25, 10]]);
    }

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

//! Shared image processing utilities.
//!
//! Common image operations used across multiple modules.

/// Convert RGB image to grayscale using ITU-R BT.601 luminance formula.
///
/// Formula: Y = 0.299R + 0.587G + 0.114B
///
/// Args:
///     image: RGB image data (row-major, 3 bytes per pixel)
///     width: Image width in pixels
///     height: Image height in pixels
///
/// Returns:
///     Grayscale image (1 byte per pixel)
pub fn rgb_to_grayscale(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut grayscale = vec![0u8; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 3;
            if idx + 2 < image.len() {
                let r = image[idx] as u32;
                let g = image[idx + 1] as u32;
                let b = image[idx + 2] as u32;
                // ITU-R BT.601 weights: 0.299R + 0.587G + 0.114B
                grayscale[y * width + x] = ((299 * r + 587 * g + 114 * b) / 1000) as u8;
            }
        }
    }

    grayscale
}

/// Compute optimal threshold using Otsu's method.
///
/// Finds the threshold that maximizes between-class variance.
///
/// Args:
///     image: Grayscale image data (1 byte per pixel)
///
/// Returns:
///     Optimal threshold value (0-255)
pub fn compute_otsu_threshold(image: &[u8]) -> u8 {
    // Build histogram
    let mut histogram = [0u32; 256];
    for &pixel in image {
        histogram[pixel as usize] += 1;
    }

    let total = image.len() as f64;
    let mut sum_total = 0.0;
    for (i, &count) in histogram.iter().enumerate() {
        sum_total += (i as f64) * (count as f64);
    }

    let mut sum_b = 0.0;
    let mut w_b = 0.0;
    let mut max_variance = 0.0;
    let mut threshold = 0u8;

    for (i, &count) in histogram.iter().enumerate() {
        w_b += count as f64;
        if w_b == 0.0 {
            continue;
        }

        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }

        sum_b += (i as f64) * (count as f64);
        let m_b = sum_b / w_b;
        let m_f = (sum_total - sum_b) / w_f;
        let variance = w_b * w_f * (m_b - m_f) * (m_b - m_f);

        if variance > max_variance {
            max_variance = variance;
            threshold = i as u8;
        }
    }

    threshold
}

/// Apply threshold to create binary image.
///
/// Pixels > threshold become 255, others become 0.
pub fn apply_threshold(image: &[u8], threshold: u8) -> Vec<u8> {
    image
        .iter()
        .map(|&x| if x > threshold { 255 } else { 0 })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_to_grayscale() {
        // White pixel (255, 255, 255) -> 255
        let white = rgb_to_grayscale(&[255, 255, 255], 1, 1);
        assert_eq!(white[0], 255);

        // Black pixel (0, 0, 0) -> 0
        let black = rgb_to_grayscale(&[0, 0, 0], 1, 1);
        assert_eq!(black[0], 0);

        // Red pixel (255, 0, 0) -> ~76 (0.299 * 255)
        let red = rgb_to_grayscale(&[255, 0, 0], 1, 1);
        assert!((red[0] as i32 - 76).abs() <= 1);
    }

    #[test]
    fn test_compute_otsu_threshold() {
        // Create bimodal histogram (half black, half white)
        let mut image = vec![0u8; 100];
        for i in 50..100 {
            image[i] = 255;
        }

        let threshold = compute_otsu_threshold(&image);
        let binary = apply_threshold(&image, threshold);

        // Should split roughly in the middle
        let white_count = binary.iter().filter(|&&x| x == 255).count();
        assert!(
            white_count > 40 && white_count < 60,
            "white_count was {}",
            white_count
        );
    }

    #[test]
    fn test_apply_threshold() {
        let image = vec![100, 150, 200, 50];
        let binary = apply_threshold(&image, 125);
        assert_eq!(binary, vec![0, 255, 255, 0]);
    }
}

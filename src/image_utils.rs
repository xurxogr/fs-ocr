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
}

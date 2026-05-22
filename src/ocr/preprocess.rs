//! Image preprocessing for OCR.
//!
//! Shared preprocessing functions used by all OCR backends.

use crate::image_utils;

/// Preprocess an image for OCR.
///
/// Applies:
/// 1. Grayscale conversion (if needed)
/// 2. Upscaling (2x for better OCR accuracy)
/// 3. Binary thresholding (Otsu's method)
/// 4. Inversion (white text on black -> black on white)
pub fn preprocess_for_ocr(
    image: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    upscale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Step 1: Convert to grayscale if needed
    let grayscale = if channels == 3 {
        image_utils::rgb_to_grayscale(image, width, height)
    } else {
        image.to_vec()
    };

    // Step 2: Upscale
    let new_width = (width as f64 * upscale_factor) as usize;
    let new_height = (height as f64 * upscale_factor) as usize;
    let upscaled = upscale_bilinear(&grayscale, width, height, new_width, new_height);

    // Step 3: Apply Otsu's threshold to create binary image
    let threshold = image_utils::compute_otsu_threshold(&upscaled);
    let binary = image_utils::apply_threshold(&upscaled, threshold);

    // Step 4: Invert (white text on black background -> black on white)
    let inverted: Vec<u8> = binary.iter().map(|&x| 255 - x).collect();

    (inverted, new_width, new_height)
}

/// Preprocess quantity composite image for OCR (Python-style).
///
/// Matches Python's stockpile_detector._build_quantity_composite_image:
/// 1. Upscale by 2/scale_factor
/// 2. Fixed threshold 120 with BINARY_INV
/// 3. Morphological close (2x2 kernel)
/// 4. Invert -> Erode -> Invert (thin text for better OCR)
pub fn preprocess_quantity_composite(
    image: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    scale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Default upscale: 2/scale_factor (matches Python)
    let upscale_factor = 2.0 / scale_factor;
    preprocess_quantity_with_upscale(image, width, height, channels, upscale_factor)
}

/// Preprocess quantity image with explicit upscale factor.
/// Use upscale_factor=1.0 for no upscaling.
pub fn preprocess_quantity_with_upscale(
    image: &[u8],
    width: usize,
    height: usize,
    channels: usize,
    upscale_factor: f64,
) -> (Vec<u8>, usize, usize) {
    // Step 1: Convert to grayscale if needed
    let grayscale = if channels == 3 {
        image_utils::rgb_to_grayscale(image, width, height)
    } else {
        image.to_vec()
    };

    // Step 2: Upscale (or skip if factor is 1.0)
    let (processed, new_width, new_height) = if (upscale_factor - 1.0).abs() < 0.01 {
        // No upscale needed
        (grayscale, width, height)
    } else {
        let new_w = (width as f64 * upscale_factor) as usize;
        let new_h = (height as f64 * upscale_factor) as usize;
        let upscaled = upscale_bilinear(&grayscale, width, height, new_w, new_h);
        (upscaled, new_w, new_h)
    };

    // Step 3: Fixed threshold 120 with BINARY_INV (pixels < 120 become 255)
    let binary: Vec<u8> = processed
        .iter()
        .map(|&x| if x < 120 { 255 } else { 0 })
        .collect();

    // Step 4: Morphological close (dilate then erode) with 2x2 kernel
    let dilated = dilate_2x2(&binary, new_width, new_height);
    let closed = erode_2x2(&dilated, new_width, new_height);

    // Step 5: Invert -> Erode -> Invert (thin text)
    let inverted: Vec<u8> = closed.iter().map(|&x| 255 - x).collect();
    let eroded = erode_2x2(&inverted, new_width, new_height);
    let final_img: Vec<u8> = eroded.iter().map(|&x| 255 - x).collect();

    (final_img, new_width, new_height)
}

/// Dilate with 2x2 kernel (max filter).
pub fn dilate_2x2(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut max_val = 0u8;
            for dy in 0..2 {
                for dx in 0..2 {
                    let ny = (y + dy).min(height - 1);
                    let nx = (x + dx).min(width - 1);
                    max_val = max_val.max(image[ny * width + nx]);
                }
            }
            result[y * width + x] = max_val;
        }
    }
    result
}

/// Erode with 2x2 kernel (min filter).
pub fn erode_2x2(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut result = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut min_val = 255u8;
            for dy in 0..2 {
                for dx in 0..2 {
                    let ny = (y + dy).min(height - 1);
                    let nx = (x + dx).min(width - 1);
                    min_val = min_val.min(image[ny * width + nx]);
                }
            }
            result[y * width + x] = min_val;
        }
    }
    result
}

/// Bilinear upscaling.
pub fn upscale_bilinear(
    image: &[u8],
    src_width: usize,
    src_height: usize,
    dst_width: usize,
    dst_height: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; dst_width * dst_height];

    let x_ratio = src_width as f64 / dst_width as f64;
    let y_ratio = src_height as f64 / dst_height as f64;

    for y in 0..dst_height {
        for x in 0..dst_width {
            // Sample at pixel centers (PIL/OpenCV convention): map the centre of
            // each destination pixel back to source space. The naive `x * ratio`
            // mapping biases sampling by half a pixel, which smears thin strokes
            // on upscale — enough to drop the `氵` radical of `海` (read as `每`).
            let src_x = ((x as f64 + 0.5) * x_ratio - 0.5).max(0.0);
            let src_y = ((y as f64 + 0.5) * y_ratio - 0.5).max(0.0);

            let x0 = src_x.floor() as usize;
            let y0 = src_y.floor() as usize;
            let x1 = (x0 + 1).min(src_width - 1);
            let y1 = (y0 + 1).min(src_height - 1);

            let x_diff = src_x - x0 as f64;
            let y_diff = src_y - y0 as f64;

            let p00 = image[y0 * src_width + x0] as f64;
            let p10 = image[y0 * src_width + x1] as f64;
            let p01 = image[y1 * src_width + x0] as f64;
            let p11 = image[y1 * src_width + x1] as f64;

            let value = p00 * (1.0 - x_diff) * (1.0 - y_diff)
                + p10 * x_diff * (1.0 - y_diff)
                + p01 * (1.0 - x_diff) * y_diff
                + p11 * x_diff * y_diff;

            result[y * dst_width + x] = value.round() as u8;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess_for_ocr() {
        let rgb = vec![128u8; 10 * 10 * 3];
        let (processed, new_w, new_h) = preprocess_for_ocr(&rgb, 10, 10, 3, 2.0);

        assert_eq!(new_w, 20);
        assert_eq!(new_h, 20);
        assert_eq!(processed.len(), 20 * 20);
    }

    #[test]
    fn test_upscale_bilinear() {
        // Simple 2x2 image
        let image = vec![0u8, 255, 255, 0];
        let upscaled = upscale_bilinear(&image, 2, 2, 4, 4);

        assert_eq!(upscaled.len(), 16);
        // Corner values should be preserved
        assert_eq!(upscaled[0], 0);
        assert_eq!(upscaled[3], 255);
    }

    #[test]
    fn test_dilate_erode() {
        // 3x3 image with center pixel = 255
        // Index layout:
        // 0 1 2
        // 3 4 5
        // 6 7 8
        let image = vec![0, 0, 0, 0, 255, 0, 0, 0, 0];
        let dilated = dilate_2x2(&image, 3, 3);
        // 2x2 kernel dilates by looking at (y+dy, x+dx) where dy,dx in {0,1}
        // So the 255 at (1,1) affects positions where kernel window includes (1,1):
        // - (0,0): window covers (0,0),(0,1),(1,0),(1,1) -> includes 255 -> 255
        // - (0,1): window covers (0,1),(0,2),(1,1),(1,2) -> includes 255 -> 255
        // - (1,0): window covers (1,0),(1,1),(2,0),(2,1) -> includes 255 -> 255
        // - (1,1): window covers (1,1),(1,2),(2,1),(2,2) -> includes 255 -> 255
        assert_eq!(dilated[0], 255, "position (0,0)");
        assert_eq!(dilated[1], 255, "position (0,1)");
        assert_eq!(dilated[3], 255, "position (1,0)");
        assert_eq!(dilated[4], 255, "position (1,1)");
    }
}

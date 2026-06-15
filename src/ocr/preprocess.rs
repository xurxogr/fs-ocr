//! Image preprocessing for OCR.
//!
//! Shared preprocessing functions used by all OCR backends.

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
    fn test_upscale_bilinear() {
        // Simple 2x2 image
        let image = vec![0u8, 255, 255, 0];
        let upscaled = upscale_bilinear(&image, 2, 2, 4, 4);

        assert_eq!(upscaled.len(), 16);
        // Corner values should be preserved
        assert_eq!(upscaled[0], 0);
        assert_eq!(upscaled[3], 255);
    }
}

//! Perceptual hashing for fast template filtering.
//!
//! Uses an average hash (aHash) algorithm matching Python's implementation:
//! 1. Convert to grayscale using OpenCV-compatible formula
//! 2. Resize image to 8x8 using INTER_AREA interpolation
//! 3. Compute average pixel value
//! 4. Generate 64-bit hash based on pixels strictly above average
//!
//! Hamming distance between two hashes indicates visual similarity.
//! Lower distance = more similar images.

/// Size of the hash grid (8x8 = 64 bits).
const HASH_SIZE: usize = 8;

/// Compute perceptual hash for a BGR image (matches Python/OpenCV exactly).
///
/// This implementation matches Python's compute_icon_phash():
/// - cv2.cvtColor(icon_image, cv2.COLOR_BGR2GRAY) for grayscale
/// - cv2.resize(gray, (8, 8), interpolation=cv2.INTER_AREA) for resize
/// - (img_resized > avg) for threshold (strictly greater, not >=)
///
/// Args:
///     image: BGR image data (row-major, 3 bytes per pixel)
///     width: Image width
///     height: Image height
///
/// Returns:
///     64-bit perceptual hash
pub fn compute_phash(image: &[u8], width: usize, height: usize) -> u64 {
    if image.is_empty() || width == 0 || height == 0 {
        return 0;
    }

    // Step 1: Convert BGR to grayscale (same formula as cv2.COLOR_BGR2GRAY)
    let grayscale = bgr_to_grayscale(image, width, height);

    // Step 2: Resize to 8x8 using INTER_AREA interpolation (matches cv2.resize)
    let resized = resize_inter_area(&grayscale, width, height, HASH_SIZE, HASH_SIZE);

    // Step 3: Compute average (using f64 for precision like numpy.mean())
    let sum: f64 = resized.iter().map(|&x| x as f64).sum();
    let avg = sum / 64.0;

    // Step 4: Generate hash (1 if pixel > avg, 0 otherwise)
    // IMPORTANT: Python uses strictly greater (>), not greater-or-equal (>=)
    let mut hash: u64 = 0;
    for (i, &pixel) in resized.iter().enumerate() {
        if (pixel as f64) > avg {
            hash |= 1 << (63 - i);
        }
    }

    hash
}

/// Convert BGR image to grayscale using OpenCV formula.
///
/// OpenCV's COLOR_BGR2GRAY uses: Y = 0.299*R + 0.587*G + 0.114*B
/// with integer math: Y = (299*R + 587*G + 114*B) / 1000
fn bgr_to_grayscale(image: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut grayscale = vec![0u8; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) * 3;
            if idx + 2 < image.len() {
                let b = image[idx] as u32;
                let g = image[idx + 1] as u32;
                let r = image[idx + 2] as u32;
                // OpenCV's formula: Y = 0.299R + 0.587G + 0.114B
                let gray = (299 * r + 587 * g + 114 * b) / 1000;
                grayscale[y * width + x] = gray as u8;
            }
        }
    }

    grayscale
}

/// Resize grayscale image using INTER_AREA interpolation (matches cv2.resize).
///
/// INTER_AREA computes each output pixel as a weighted average of all input
/// pixels that overlap with that output pixel's corresponding area in the
/// source image.
fn resize_inter_area(
    image: &[u8],
    in_width: usize,
    in_height: usize,
    out_width: usize,
    out_height: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; out_width * out_height];

    if in_width == 0 || in_height == 0 {
        return result;
    }

    let scale_x = in_width as f64 / out_width as f64;
    let scale_y = in_height as f64 / out_height as f64;

    for out_y in 0..out_height {
        for out_x in 0..out_width {
            // Compute the input area that maps to this output pixel
            let in_x_start = out_x as f64 * scale_x;
            let in_x_end = (out_x + 1) as f64 * scale_x;
            let in_y_start = out_y as f64 * scale_y;
            let in_y_end = (out_y + 1) as f64 * scale_y;

            let mut sum = 0.0f64;
            let mut total_weight = 0.0f64;

            // Iterate over all input pixels that could overlap
            let ix_start = in_x_start.floor() as usize;
            let ix_end = (in_x_end.ceil() as usize).min(in_width);
            let iy_start = in_y_start.floor() as usize;
            let iy_end = (in_y_end.ceil() as usize).min(in_height);

            for iy in iy_start..iy_end {
                for ix in ix_start..ix_end {
                    // Compute the overlap area between this input pixel and the output area
                    let x_overlap_start = in_x_start.max(ix as f64);
                    let x_overlap_end = in_x_end.min((ix + 1) as f64);
                    let y_overlap_start = in_y_start.max(iy as f64);
                    let y_overlap_end = in_y_end.min((iy + 1) as f64);

                    let x_overlap = (x_overlap_end - x_overlap_start).max(0.0);
                    let y_overlap = (y_overlap_end - y_overlap_start).max(0.0);
                    let weight = x_overlap * y_overlap;

                    if weight > 0.0 {
                        let pixel_value = image[iy * in_width + ix] as f64;
                        sum += pixel_value * weight;
                        total_weight += weight;
                    }
                }
            }

            result[out_y * out_width + out_x] = if total_weight > 0.0 {
                (sum / total_weight).round() as u8
            } else {
                0
            };
        }
    }

    result
}

/// Compute Hamming distance between two pHash values.
///
/// The Hamming distance is the number of bits that differ between two values.
#[inline]
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Filter candidates by pHash distance.
///
/// Returns indices of candidates within the threshold, sorted by distance.
pub fn filter_by_phash(
    icon_phash: u64,
    candidate_phashes: &[u64],
    threshold: u32,
    max_candidates: usize,
) -> Vec<(usize, u32)> {
    let mut matches: Vec<(usize, u32)> = candidate_phashes
        .iter()
        .enumerate()
        .map(|(i, &phash)| (i, hamming_distance(icon_phash, phash)))
        .filter(|(_, dist)| *dist <= threshold)
        .collect();

    // Sort by distance (ascending)
    matches.sort_by_key(|(_, dist)| *dist);

    // Take top N
    matches.truncate(max_candidates);

    matches
}

/// Compute pHash from grayscale image data (matches Python exactly).
///
/// Args:
///     grayscale: 8-bit grayscale image data
///     width: Image width
///     height: Image height
///
/// Returns:
///     64-bit perceptual hash
pub fn compute_phash_grayscale(grayscale: &[u8], width: usize, height: usize) -> u64 {
    if grayscale.is_empty() || width == 0 || height == 0 {
        return 0;
    }

    // Step 1: Resize to 8x8 using INTER_AREA interpolation
    let resized = resize_inter_area(grayscale, width, height, HASH_SIZE, HASH_SIZE);

    // Step 2: Compute average (using f64 for precision like numpy.mean())
    let sum: f64 = resized.iter().map(|&x| x as f64).sum();
    let avg = sum / 64.0;

    // Step 3: Generate hash (1 if pixel > avg, 0 otherwise)
    // IMPORTANT: Python uses strictly greater (>), not greater-or-equal (>=)
    let mut hash: u64 = 0;
    for (i, &pixel) in resized.iter().enumerate() {
        if (pixel as f64) > avg {
            hash |= 1 << (63 - i);
        }
    }

    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hamming_distance() {
        assert_eq!(hamming_distance(0, 0), 0);
        assert_eq!(hamming_distance(0b1010, 0b0101), 4);
        assert_eq!(hamming_distance(0xFFFF_FFFF_FFFF_FFFF, 0), 64);
        assert_eq!(hamming_distance(0xFF, 0xFE), 1); // Only last bit differs
    }

    #[test]
    fn test_filter_by_phash() {
        let icon = 0b1010u64;
        let candidates = vec![0b1010, 0b0000, 0b1111, 0b1011];

        let matches = filter_by_phash(icon, &candidates, 20, 10);

        assert_eq!(matches.len(), 4); // All within threshold
        assert_eq!(matches[0], (0, 0)); // Exact match first
        assert_eq!(matches[1], (3, 1)); // 1 bit different
    }

    #[test]
    fn test_compute_phash_empty() {
        assert_eq!(compute_phash(&[], 0, 0), 0);
        assert_eq!(compute_phash(&[0; 3], 0, 0), 0);
    }

    #[test]
    fn test_compute_phash_uniform() {
        // All white image (255, 255, 255) in BGR format
        let white: Vec<u8> = vec![255; 64 * 64 * 3];
        let hash_white = compute_phash(&white, 64, 64);

        // All black image (0, 0, 0)
        let black: Vec<u8> = vec![0; 64 * 64 * 3];
        let hash_black = compute_phash(&black, 64, 64);

        // Uniform images: all pixels equal average, so with ">" comparison, all bits are 0
        assert_eq!(hash_white, 0);
        assert_eq!(hash_black, 0);
    }

    #[test]
    fn test_compute_phash_gradient() {
        // Create a horizontal gradient (left=black, right=white) in BGR format
        let mut gradient = vec![0u8; 64 * 64 * 3];
        for y in 0..64 {
            for x in 0..64 {
                let idx = (y * 64 + x) * 3;
                let val = (x * 255 / 63) as u8;
                gradient[idx] = val; // B
                gradient[idx + 1] = val; // G
                gradient[idx + 2] = val; // R
            }
        }

        let hash = compute_phash(&gradient, 64, 64);

        // Hash should have some bits set (not all 0 or all 1)
        // Left half below average -> 0, right half above -> 1
        assert_ne!(hash, 0);
        assert_ne!(hash, 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn test_compute_phash_similarity() {
        // Create two similar images with slight difference
        let img1 = vec![128u8; 64 * 64 * 3];
        let mut img2 = img1.clone();

        // Change a small region in img2
        for i in 0..100 {
            img2[i] = 180;
        }

        let hash1 = compute_phash(&img1, 64, 64);
        let hash2 = compute_phash(&img2, 64, 64);

        // Similar images should have low Hamming distance
        let distance = hamming_distance(hash1, hash2);
        assert!(distance < 10, "Distance {} should be small", distance);
    }

    #[test]
    fn test_compute_phash_grayscale() {
        let gray: Vec<u8> = vec![128; 64 * 64];
        let hash = compute_phash_grayscale(&gray, 64, 64);
        // Uniform gray image - with ">" comparison, all pixels equal average, so hash = 0
        assert_eq!(hash, 0);
    }

    #[test]
    fn test_resize_inter_area_exact_divisor() {
        // 64x64 -> 8x8 means each 8x8 block maps to one output pixel
        let mut img = vec![0u8; 64 * 64];
        // Set top-left 8x8 block to 200
        for y in 0..8 {
            for x in 0..8 {
                img[y * 64 + x] = 200;
            }
        }
        // Set next 8x8 block (top row, second column) to 100
        for y in 0..8 {
            for x in 8..16 {
                img[y * 64 + x] = 100;
            }
        }

        let resized = resize_inter_area(&img, 64, 64, 8, 8);

        assert_eq!(resized[0], 200); // Top-left block
        assert_eq!(resized[1], 100); // Second block in top row
        assert_eq!(resized[8], 0); // First block in second row (was 0)
    }

    #[test]
    fn test_resize_inter_area_non_divisor() {
        // 42x42 -> 8x8 tests proper area weighting (42 not divisible by 8)
        let img = vec![100u8; 42 * 42];
        let resized = resize_inter_area(&img, 42, 42, 8, 8);

        // All pixels same value -> all output pixels should be same value
        for pixel in resized.iter() {
            assert_eq!(*pixel, 100);
        }
    }

    #[test]
    fn test_bgr_to_grayscale() {
        // Pure red (BGR: 0, 0, 255) -> Y = 0.299 * 255 = 76
        let red = vec![0u8, 0, 255];
        let gray = bgr_to_grayscale(&red, 1, 1);
        assert_eq!(gray[0], 76);

        // Pure green (BGR: 0, 255, 0) -> Y = 0.587 * 255 = 149
        let green = vec![0u8, 255, 0];
        let gray = bgr_to_grayscale(&green, 1, 1);
        assert_eq!(gray[0], 149);

        // Pure blue (BGR: 255, 0, 0) -> Y = 0.114 * 255 = 29
        let blue = vec![255u8, 0, 0];
        let gray = bgr_to_grayscale(&blue, 1, 1);
        assert_eq!(gray[0], 29);
    }
}

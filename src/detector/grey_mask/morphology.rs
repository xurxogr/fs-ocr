//! Binary-mask morphology (dilate/erode) and connected-component contour extraction.

use rayon::prelude::*;

use super::BoundingRect;

/// Parallel dilation operation using separable 1D passes.
/// Much faster than naive 2D kernel approach.
pub(super) fn dilate(image: &[u8], width: usize, height: usize, kernel_size: usize) -> Vec<u8> {
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
pub(super) fn erode(image: &[u8], width: usize, height: usize, kernel_size: usize) -> Vec<u8> {
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
pub(super) fn find_contours(mask: &[u8], width: usize, height: usize) -> Vec<BoundingRect> {
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
}

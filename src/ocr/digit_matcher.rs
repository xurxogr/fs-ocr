//! Template-based digit recognition for game fonts.
//!
//! Uses pre-computed templates for 0-9 to recognize quantities.
//! Optimized for the Renner font used in Foxhole.


/// Template height (normalized to 24px at 2160p scale).
const TEMPLATE_HEIGHT: usize = 24;

/// Minimum match score to accept a digit (0.0-1.0).
const MIN_MATCH_SCORE: f64 = 0.6;

/// Digit template with bit-packed data.
struct DigitTemplate {
    digit: char,
    width: usize,
    /// Bit-packed rows (MSB first, 8 pixels per byte).
    data: &'static [u8],
}

// Templates extracted from 2160p Renner font
const TEMPLATES: &[DigitTemplate] = &[
    DigitTemplate {
        digit: '0',
        width: 17,
        data: &[
            0x03, 0xe0, 0x00, 0x0f, 0xf8, 0x00, 0x1f, 0xfc, 0x00, 0x3e, 0x3e, 0x00,
            0x38, 0x0e, 0x00, 0x78, 0x0f, 0x00, 0x70, 0x07, 0x00, 0x70, 0x07, 0x80,
            0xf0, 0x03, 0x80, 0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80,
            0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80, 0xf0, 0x03, 0x80,
            0x70, 0x07, 0x00, 0x70, 0x07, 0x00, 0x78, 0x0f, 0x00, 0x3c, 0x1e, 0x00,
            0x1f, 0x7c, 0x00, 0x0f, 0xfc, 0x00, 0x07, 0xf0, 0x00, 0x00, 0x80, 0x00,
        ],
    },
    DigitTemplate {
        digit: '1',
        width: 8,
        data: &[
            0x01, 0x3f, 0xff, 0xff, 0xe7, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
            0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x00,
        ],
    },
    DigitTemplate {
        digit: '2',
        width: 16,
        data: &[
            0x03, 0xe0, 0x0f, 0xf8, 0x1f, 0xfc, 0x3c, 0x1e, 0x78, 0x0e, 0x70, 0x0e,
            0x70, 0x0e, 0x70, 0x0e, 0x00, 0x0e, 0x00, 0x1e, 0x00, 0x1c, 0x00, 0x3c,
            0x00, 0x78, 0x00, 0xf0, 0x01, 0xe0, 0x03, 0xc0, 0x07, 0x80, 0x0f, 0x00,
            0x0e, 0x00, 0x1e, 0x00, 0x3f, 0xfe, 0x7f, 0xff, 0xff, 0xff, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '3',
        width: 14,
        data: &[
            0x07, 0x80, 0x1f, 0xe0, 0x3f, 0xf0, 0x78, 0x78, 0x70, 0x38, 0x70, 0x38,
            0x00, 0x38, 0x00, 0x38, 0x00, 0x38, 0x00, 0x78, 0x03, 0xf0, 0x03, 0xe0,
            0x03, 0xf0, 0x00, 0x78, 0x00, 0x3c, 0x00, 0x1c, 0x00, 0x1c, 0xe0, 0x1c,
            0xe0, 0x1c, 0xf0, 0x3c, 0x78, 0xf8, 0x3f, 0xf0, 0x1f, 0xe0, 0x02, 0x00,
        ],
    },
    DigitTemplate {
        digit: '4',
        width: 17,
        data: &[
            0x00, 0x04, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00,
            0x00, 0x3c, 0x00, 0x00, 0x7c, 0x00, 0x00, 0x7c, 0x00, 0x00, 0xfc, 0x00,
            0x01, 0xfc, 0x00, 0x01, 0xdc, 0x00, 0x03, 0x9c, 0x00, 0x07, 0x1c, 0x00,
            0x0f, 0x1c, 0x00, 0x0e, 0x1c, 0x00, 0x1c, 0x1c, 0x00, 0x3c, 0x1c, 0x00,
            0x38, 0x1c, 0x00, 0x7f, 0xff, 0x80, 0xff, 0xff, 0x80, 0xff, 0xff, 0x80,
            0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00,
        ],
    },
    DigitTemplate {
        digit: '5',
        width: 16,
        data: &[
            0x07, 0xfe, 0x07, 0xff, 0x0f, 0xff, 0x0e, 0x00, 0x0e, 0x00, 0x0e, 0x00,
            0x1e, 0x00, 0x1c, 0x00, 0x1f, 0xf0, 0x1f, 0xf8, 0x3f, 0xfc, 0x30, 0x1e,
            0x00, 0x0e, 0x00, 0x0f, 0x00, 0x07, 0x00, 0x07, 0x00, 0x07, 0x60, 0x0f,
            0xf0, 0x0f, 0xf8, 0x1e, 0x7f, 0xfe, 0x3f, 0xfc, 0x0f, 0xf0, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '6',
        width: 16,
        data: &[
            0x00, 0x70, 0x00, 0xf0, 0x00, 0xe0, 0x01, 0xe0, 0x03, 0xc0, 0x07, 0x80,
            0x07, 0x00, 0x0f, 0x00, 0x1f, 0xe0, 0x1f, 0xf8, 0x3f, 0xfc, 0x78, 0x1e,
            0x70, 0x0e, 0xf0, 0x0e, 0xe0, 0x0f, 0xe0, 0x07, 0xe0, 0x07, 0xe0, 0x0f,
            0x70, 0x0e, 0x78, 0x1e, 0x3e, 0x7c, 0x1f, 0xf8, 0x0f, 0xf0, 0x01, 0x00,
        ],
    },
    DigitTemplate {
        digit: '7',
        width: 15,
        data: &[
            0xff, 0xfe, 0xff, 0xfe, 0xff, 0xfe, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x38,
            0x00, 0x38, 0x00, 0x78, 0x00, 0x70, 0x00, 0xf0, 0x00, 0xe0, 0x01, 0xe0,
            0x01, 0xc0, 0x03, 0xc0, 0x03, 0x80, 0x07, 0x80, 0x07, 0x00, 0x0f, 0x00,
            0x0e, 0x00, 0x0e, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x38, 0x00, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '8',
        width: 14,
        data: &[
            0x0f, 0x80, 0x1f, 0xe0, 0x3f, 0xf0, 0x78, 0x78, 0x70, 0x38, 0xe0, 0x38,
            0xe0, 0x38, 0xf0, 0x38, 0x70, 0x78, 0x7c, 0xf0, 0x3f, 0xe0, 0x1f, 0xe0,
            0x7f, 0xf0, 0x70, 0x78, 0xe0, 0x38, 0xe0, 0x1c, 0xe0, 0x1c, 0xe0, 0x1c,
            0xe0, 0x3c, 0xf0, 0x38, 0x7c, 0xf8, 0x3f, 0xf0, 0x1f, 0xe0, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '9',
        width: 15,
        data: &[
            0x07, 0xc0, 0x1f, 0xf0, 0x3f, 0xf8, 0x78, 0x3c, 0xf0, 0x1e, 0xe0, 0x0e,
            0xe0, 0x0e, 0xe0, 0x0e, 0xe0, 0x0e, 0xe0, 0x0e, 0xe0, 0x0e, 0x70, 0x1e,
            0x78, 0x3c, 0x3f, 0xf8, 0x1f, 0xf8, 0x00, 0xf0, 0x01, 0xe0, 0x01, 0xc0,
            0x03, 0xc0, 0x07, 0x80, 0x0f, 0x00, 0x0f, 0x00, 0x1e, 0x00, 0x00, 0x00,
        ],
    },
    // Template for 'k' (thousands suffix)
    DigitTemplate {
        digit: 'k',
        width: 11,
        data: &[
            0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00,
            0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0xe0, 0xe1, 0xc0,
            0xe3, 0x80, 0xe7, 0x00, 0xee, 0x00, 0xfc, 0x00, 0xf8, 0x00, 0xfc, 0x00,
            0xfe, 0x00, 0xef, 0x00, 0xe7, 0x80, 0xe3, 0x80, 0xe1, 0xc0, 0xe1, 0xe0,
        ],
    },
    // Template for '+' (overflow indicator, 16px wide padded to 24px height)
    DigitTemplate {
        digit: '+',
        width: 16,
        data: &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x80, 0x03, 0x80,
            0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0xff, 0xfe, 0xff, 0xff,
            0xff, 0xff, 0x03, 0xc0, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80,
            0x03, 0x80, 0x03, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    },
];

/// Get pixel value from bit-packed template data.
#[inline]
fn get_template_pixel(template: &DigitTemplate, x: usize, y: usize) -> bool {
    if x >= template.width || y >= TEMPLATE_HEIGHT {
        return false;
    }
    let bytes_per_row = (template.width + 7) / 8;
    let byte_idx = y * bytes_per_row + x / 8;
    let bit_idx = 7 - (x % 8);
    (template.data[byte_idx] >> bit_idx) & 1 == 1
}

/// Compute match score between image region and template.
/// Returns score 0.0-1.0 (1.0 = perfect match).
fn compute_match_score(
    image: &[u8],
    img_width: usize,
    img_height: usize,
    template: &DigitTemplate,
    _scale: f64,
) -> f64 {
    // Use template dimensions directly (templates are for 2160p)
    let tw = template.width;
    let th = TEMPLATE_HEIGHT;

    // Resize image region to template size for comparison
    // Use simple nearest-neighbor scaling
    let mut template_matches = 0u32;
    let mut template_total = 0u32;
    let mut image_white = 0u32;

    for ty in 0..th {
        for tx in 0..tw {
            // Map template coord to image coord
            let ix = (tx * img_width) / tw;
            let iy = (ty * img_height) / th;

            let template_pixel = get_template_pixel(template, tx, ty);
            let image_pixel = if ix < img_width && iy < img_height {
                image[iy * img_width + ix] > 128
            } else {
                false
            };

            if image_pixel {
                image_white += 1;
            }

            // Score based on matching foreground pixels
            if template_pixel && image_pixel {
                template_matches += 1;
            }
            if template_pixel {
                template_total += 1;
            }
        }
    }

    if template_total == 0 || image_white == 0 {
        return 0.0;
    }

    // Compute F1-like score: balance precision and recall
    let recall = template_matches as f64 / template_total as f64;
    let precision = template_matches as f64 / image_white as f64;

    if recall + precision == 0.0 {
        0.0
    } else {
        2.0 * recall * precision / (recall + precision)
    }
}

/// Find connected components in a binary image.
/// Returns list of (x, y, width, height) bounding boxes sorted by x.
fn find_components(image: &[u8], width: usize, height: usize) -> Vec<(usize, usize, usize, usize)> {
    let mut labels = vec![0u32; width * height];
    let mut next_label = 1u32;
    let mut equivalences: Vec<u32> = vec![0]; // Union-find

    // First pass: assign labels
    for y in 0..height {
        for x in 0..width {
            if image[y * width + x] < 128 {
                continue; // Background
            }

            let mut neighbors = Vec::new();

            // Check left
            if x > 0 && labels[y * width + x - 1] > 0 {
                neighbors.push(labels[y * width + x - 1]);
            }
            // Check top
            if y > 0 && labels[(y - 1) * width + x] > 0 {
                neighbors.push(labels[(y - 1) * width + x]);
            }

            if neighbors.is_empty() {
                labels[y * width + x] = next_label;
                equivalences.push(next_label);
                next_label += 1;
            } else {
                let min_label = *neighbors.iter().min().unwrap();
                labels[y * width + x] = min_label;

                // Union all neighbors
                for &n in &neighbors {
                    union(&mut equivalences, min_label, n);
                }
            }
        }
    }

    // Find root labels
    fn find(eq: &mut [u32], x: u32) -> u32 {
        if eq[x as usize] != x {
            eq[x as usize] = find(eq, eq[x as usize]);
        }
        eq[x as usize]
    }

    fn union(eq: &mut [u32], a: u32, b: u32) {
        let ra = find(eq, a);
        let rb = find(eq, b);
        if ra != rb {
            eq[ra as usize] = rb;
        }
    }

    // Second pass: compute bounding boxes per component
    let mut boxes: std::collections::HashMap<u32, (usize, usize, usize, usize)> =
        std::collections::HashMap::new();

    for y in 0..height {
        for x in 0..width {
            let label = labels[y * width + x];
            if label == 0 {
                continue;
            }
            let root = find(&mut equivalences, label);

            boxes
                .entry(root)
                .and_modify(|(min_x, min_y, max_x, max_y)| {
                    *min_x = (*min_x).min(x);
                    *min_y = (*min_y).min(y);
                    *max_x = (*max_x).max(x);
                    *max_y = (*max_y).max(y);
                })
                .or_insert((x, y, x, y));
        }
    }

    // Convert to (x, y, w, h) and sort by x
    let mut result: Vec<_> = boxes
        .values()
        .filter(|(min_x, min_y, max_x, max_y)| {
            let w = max_x - min_x + 1;
            let h = max_y - min_y + 1;
            // Filter noise: minimum size and area
            w >= 3 && h >= 10 && (w * h) >= 50
        })
        .map(|(min_x, min_y, max_x, max_y)| (*min_x, *min_y, max_x - min_x + 1, max_y - min_y + 1))
        .collect();

    result.sort_by_key(|&(x, _, _, _)| x);
    result
}

/// Recognize a single digit from an image region.
fn recognize_digit(image: &[u8], width: usize, height: usize, scale: f64) -> Option<char> {
    let mut best_digit = None;
    let mut best_score = MIN_MATCH_SCORE;

    // Scale factor is used for width penalty calculation

    for template in TEMPLATES {
        // Check aspect ratio compatibility
        let scaled_template_width = (template.width as f64 * scale) as usize;

        // Width ratio: penalize if image width is very different from expected template width
        let width_ratio = width as f64 / scaled_template_width.max(1) as f64;
        let width_penalty = if width_ratio < 0.5 || width_ratio > 2.0 {
            0.5 // Heavy penalty for very different widths
        } else if width_ratio < 0.7 || width_ratio > 1.4 {
            0.85 // Moderate penalty
        } else {
            1.0 // No penalty
        };

        let score = compute_match_score(image, width, height, template, scale) * width_penalty;
        if score > best_score {
            best_score = score;
            best_digit = Some(template.digit);
        }
    }

    best_digit
}

/// Recognize a quantity from a box image.
///
/// # Arguments
/// * `image` - Grayscale image data
/// * `width` - Image width
/// * `height` - Image height
/// * `scale` - Scale factor (1.0 for 2160p)
///
/// # Returns
/// Recognized quantity as i32, or -1 if recognition failed.
pub fn recognize_quantity(image: &[u8], width: usize, height: usize, scale: f64) -> i32 {
    // Threshold to binary (white text on black background)
    let binary: Vec<u8> = image.iter().map(|&v| if v > 120 { 255 } else { 0 }).collect();

    // Find connected components (individual digits)
    let components = find_components(&binary, width, height);

    if components.is_empty() {
        return -1;
    }

    // Recognize each component
    let mut digits = String::new();

    for (cx, cy, cw, ch) in components {
        // Extract component region
        let mut region = vec![0u8; cw * ch];
        for y in 0..ch {
            for x in 0..cw {
                region[y * cw + x] = binary[(cy + y) * width + (cx + x)];
            }
        }

        if let Some(digit) = recognize_digit(&region, cw, ch, scale) {
            digits.push(digit);
        }
    }

    // Parse the digit string
    if digits.is_empty() {
        return -1;
    }

    // Handle '+' suffix first (overflow indicator, e.g., "1k+" -> "1k")
    if digits.ends_with('+') {
        digits.pop();
    }

    // Handle 'k' suffix (e.g., "1k" = 1000)
    let multiplier = if digits.ends_with('k') {
        digits.pop();
        1000
    } else {
        1
    };

    digits.parse::<i32>().unwrap_or(-1) * multiplier
}

/// Recognize quantities from multiple box regions.
///
/// This is the main entry point for batch recognition.
pub fn recognize_quantities_batch(
    image: &[u8],
    width: i32,
    height: i32,
    boxes: &[(i32, i32)],
    box_width: i32,
    box_height: i32,
    scale: f64,
) -> Vec<i32> {
    let width = width as usize;
    let height = height as usize;
    let bw = box_width as usize;
    let bh = box_height as usize;

    boxes
        .iter()
        .map(|&(bx, by)| {
            let bx = bx as usize;
            let by = by as usize;

            // Bounds check
            if bx + bw > width || by + bh > height {
                return -1;
            }

            // Extract box region
            let mut region = vec![0u8; bw * bh];
            for y in 0..bh {
                for x in 0..bw {
                    let src_idx = (by + y) * width + (bx + x);
                    region[y * bw + x] = image[src_idx];
                }
            }

            recognize_quantity(&region, bw, bh, scale)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_template_pixel() {
        // Template '1' should have pixels in the vertical bar
        let template = &TEMPLATES[1]; // '1'
        assert_eq!(template.digit, '1');

        // Check some pixels are set
        let mut has_pixels = false;
        for y in 0..TEMPLATE_HEIGHT {
            for x in 0..template.width {
                if get_template_pixel(template, x, y) {
                    has_pixels = true;
                }
            }
        }
        assert!(has_pixels);
    }

    #[test]
    fn test_find_components_empty() {
        let image = vec![0u8; 10 * 10];
        let components = find_components(&image, 10, 10);
        assert!(components.is_empty());
    }

    #[test]
    fn test_find_components_single() {
        let mut image = vec![0u8; 20 * 20];
        // Draw a vertical bar
        for y in 2..18 {
            for x in 8..12 {
                image[y * 20 + x] = 255;
            }
        }
        let components = find_components(&image, 20, 20);
        assert_eq!(components.len(), 1);
    }
}

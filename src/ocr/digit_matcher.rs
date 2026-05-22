//! Template-based digit recognition for game fonts.
//!
//! Uses pre-computed templates for 0-9 to recognize quantities.
//! Optimized for the Renner font used in Foxhole.

/// Template height (normalized to 24px at 2160p scale).
const TEMPLATE_HEIGHT: usize = 24;

/// Minimum match score to accept a digit at 2160p (0.0-1.0).
const MIN_MATCH_SCORE: f64 = 0.6;

/// Minimum match score at low resolutions (e.g., 1080p) — relaxed because
/// downsampled glyphs lose detail and rarely hit the 0.6 threshold.
const MIN_MATCH_SCORE_LOW: f64 = 0.45;

/// Below this scale we use the relaxed threshold.
const LOW_SCALE_CUTOFF: f64 = 0.75;

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
            0x03, 0xe0, 0x00, 0x0f, 0xf8, 0x00, 0x1f, 0xfc, 0x00, 0x3e, 0x3e, 0x00, 0x38, 0x0e,
            0x00, 0x78, 0x0f, 0x00, 0x70, 0x07, 0x00, 0x70, 0x07, 0x80, 0xf0, 0x03, 0x80, 0xe0,
            0x03, 0x80, 0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80, 0xe0, 0x03, 0x80,
            0xe0, 0x03, 0x80, 0xf0, 0x03, 0x80, 0x70, 0x07, 0x00, 0x70, 0x07, 0x00, 0x78, 0x0f,
            0x00, 0x3c, 0x1e, 0x00, 0x1f, 0x7c, 0x00, 0x0f, 0xfc, 0x00, 0x07, 0xf0, 0x00, 0x00,
            0x80, 0x00,
        ],
    },
    DigitTemplate {
        digit: '1',
        width: 8,
        data: &[
            0x01, 0x3f, 0xff, 0xff, 0xe7, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07,
            0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x07, 0x00,
        ],
    },
    DigitTemplate {
        digit: '2',
        width: 16,
        data: &[
            0x03, 0xe0, 0x0f, 0xf8, 0x1f, 0xfc, 0x3c, 0x1e, 0x78, 0x0e, 0x70, 0x0e, 0x70, 0x0e,
            0x70, 0x0e, 0x00, 0x0e, 0x00, 0x1e, 0x00, 0x1c, 0x00, 0x3c, 0x00, 0x78, 0x00, 0xf0,
            0x01, 0xe0, 0x03, 0xc0, 0x07, 0x80, 0x0f, 0x00, 0x0e, 0x00, 0x1e, 0x00, 0x3f, 0xfe,
            0x7f, 0xff, 0xff, 0xff, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '3',
        width: 14,
        data: &[
            0x07, 0x80, 0x1f, 0xe0, 0x3f, 0xf0, 0x78, 0x78, 0x70, 0x38, 0x70, 0x38, 0x00, 0x38,
            0x00, 0x38, 0x00, 0x38, 0x00, 0x78, 0x03, 0xf0, 0x03, 0xe0, 0x03, 0xf0, 0x00, 0x78,
            0x00, 0x3c, 0x00, 0x1c, 0x00, 0x1c, 0xe0, 0x1c, 0xe0, 0x1c, 0xf0, 0x3c, 0x78, 0xf8,
            0x3f, 0xf0, 0x1f, 0xe0, 0x02, 0x00,
        ],
    },
    DigitTemplate {
        digit: '4',
        width: 17,
        data: &[
            0x00, 0x04, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x3c,
            0x00, 0x00, 0x7c, 0x00, 0x00, 0x7c, 0x00, 0x00, 0xfc, 0x00, 0x01, 0xfc, 0x00, 0x01,
            0xdc, 0x00, 0x03, 0x9c, 0x00, 0x07, 0x1c, 0x00, 0x0f, 0x1c, 0x00, 0x0e, 0x1c, 0x00,
            0x1c, 0x1c, 0x00, 0x3c, 0x1c, 0x00, 0x38, 0x1c, 0x00, 0x7f, 0xff, 0x80, 0xff, 0xff,
            0x80, 0xff, 0xff, 0x80, 0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x1c, 0x00, 0x00,
            0x1c, 0x00,
        ],
    },
    DigitTemplate {
        digit: '5',
        width: 16,
        data: &[
            0x07, 0xfe, 0x07, 0xff, 0x0f, 0xff, 0x0e, 0x00, 0x0e, 0x00, 0x0e, 0x00, 0x1e, 0x00,
            0x1c, 0x00, 0x1f, 0xf0, 0x1f, 0xf8, 0x3f, 0xfc, 0x30, 0x1e, 0x00, 0x0e, 0x00, 0x0f,
            0x00, 0x07, 0x00, 0x07, 0x00, 0x07, 0x60, 0x0f, 0xf0, 0x0f, 0xf8, 0x1e, 0x7f, 0xfe,
            0x3f, 0xfc, 0x0f, 0xf0, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '6',
        width: 16,
        data: &[
            0x00, 0x70, 0x00, 0xf0, 0x00, 0xe0, 0x01, 0xe0, 0x03, 0xc0, 0x07, 0x80, 0x07, 0x00,
            0x0f, 0x00, 0x1f, 0xe0, 0x1f, 0xf8, 0x3f, 0xfc, 0x78, 0x1e, 0x70, 0x0e, 0xf0, 0x0e,
            0xe0, 0x0f, 0xe0, 0x07, 0xe0, 0x07, 0xe0, 0x0f, 0x70, 0x0e, 0x78, 0x1e, 0x3e, 0x7c,
            0x1f, 0xf8, 0x0f, 0xf0, 0x01, 0x00,
        ],
    },
    DigitTemplate {
        digit: '7',
        width: 15,
        data: &[
            0xff, 0xfe, 0xff, 0xfe, 0xff, 0xfe, 0x00, 0x1c, 0x00, 0x1c, 0x00, 0x38, 0x00, 0x38,
            0x00, 0x78, 0x00, 0x70, 0x00, 0xf0, 0x00, 0xe0, 0x01, 0xe0, 0x01, 0xc0, 0x03, 0xc0,
            0x03, 0x80, 0x07, 0x80, 0x07, 0x00, 0x0f, 0x00, 0x0e, 0x00, 0x0e, 0x00, 0x1c, 0x00,
            0x1c, 0x00, 0x38, 0x00, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '8',
        width: 14,
        data: &[
            0x0f, 0x80, 0x1f, 0xe0, 0x3f, 0xf0, 0x78, 0x78, 0x70, 0x38, 0xe0, 0x38, 0xe0, 0x38,
            0xf0, 0x38, 0x70, 0x78, 0x7c, 0xf0, 0x3f, 0xe0, 0x1f, 0xe0, 0x7f, 0xf0, 0x70, 0x78,
            0xe0, 0x38, 0xe0, 0x1c, 0xe0, 0x1c, 0xe0, 0x1c, 0xe0, 0x3c, 0xf0, 0x38, 0x7c, 0xf8,
            0x3f, 0xf0, 0x1f, 0xe0, 0x00, 0x00,
        ],
    },
    DigitTemplate {
        digit: '9',
        width: 15,
        data: &[
            0x07, 0xc0, 0x1f, 0xf0, 0x3f, 0xf8, 0x78, 0x3c, 0xf0, 0x1e, 0xe0, 0x0e, 0xe0, 0x0e,
            0xe0, 0x0e, 0xe0, 0x0e, 0xe0, 0x0e, 0xe0, 0x0e, 0x70, 0x1e, 0x78, 0x3c, 0x3f, 0xf8,
            0x1f, 0xf8, 0x00, 0xf0, 0x01, 0xe0, 0x01, 0xc0, 0x03, 0xc0, 0x07, 0x80, 0x0f, 0x00,
            0x0f, 0x00, 0x1e, 0x00, 0x00, 0x00,
        ],
    },
    // Template for 'k' (thousands suffix)
    DigitTemplate {
        digit: 'k',
        width: 11,
        data: &[
            0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00,
            0xe0, 0x00, 0xe0, 0x00, 0xe0, 0x00, 0xe0, 0xe0, 0xe1, 0xc0, 0xe3, 0x80, 0xe7, 0x00,
            0xee, 0x00, 0xfc, 0x00, 0xf8, 0x00, 0xfc, 0x00, 0xfe, 0x00, 0xef, 0x00, 0xe7, 0x80,
            0xe3, 0x80, 0xe1, 0xc0, 0xe1, 0xe0,
        ],
    },
    // Template for '+' (overflow indicator, 16px wide padded to 24px height)
    DigitTemplate {
        digit: '+',
        width: 16,
        data: &[
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80,
            0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0xff, 0xfe, 0xff, 0xff, 0xff, 0xff, 0x03, 0xc0,
            0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0x03, 0x80, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    },
];

/// Get pixel value from bit-packed template data.
#[inline]
fn get_template_pixel(template: &DigitTemplate, x: usize, y: usize) -> bool {
    if x >= template.width || y >= TEMPLATE_HEIGHT {
        return false;
    }
    let bytes_per_row = template.width.div_ceil(8);
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
fn find_components(
    image: &[u8],
    width: usize,
    height: usize,
    scale: f64,
) -> Vec<(usize, usize, usize, usize)> {
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

    // Resolution-aware noise thresholds. At 2160p (scale=1.0) we keep the
    // original 3×10 / 50px² floor; at 1080p (scale≈0.5) this shrinks to 1×5/12.
    let s = scale.max(0.1);
    let min_w = ((3.0 * s).round() as usize).max(1);
    let min_h = ((10.0 * s).round() as usize).max(3);
    let min_area = ((50.0 * s * s).round() as usize).max(8);

    // Convert to (x, y, w, h) and sort by x
    let raw: Vec<_> = boxes
        .values()
        .filter(|(min_x, min_y, max_x, max_y)| {
            let w = max_x - min_x + 1;
            let h = max_y - min_y + 1;
            w >= min_w && h >= min_h && (w * h) >= min_area
        })
        .map(|(min_x, min_y, max_x, max_y)| (*min_x, *min_y, max_x - min_x + 1, max_y - min_y + 1))
        .collect();

    // Split components that look like several touching digits (e.g. "57" at
    // 1080p where the top bars of 5 and 7 share a row of foreground pixels).
    let mut result = Vec::with_capacity(raw.len());
    for comp in raw {
        split_merged_component(image, width, comp, scale, &mut result);
    }

    result.sort_by_key(|&(x, _, _, _)| x);
    result
}

/// Recursively split a wide component along vertical valleys (columns with
/// few foreground pixels). Digits are taller than wide, so a component with
/// `w > h * SPLIT_ASPECT` is suspected to be two merged glyphs.
fn split_merged_component(
    image: &[u8],
    width: usize,
    component: (usize, usize, usize, usize),
    scale: f64,
    out: &mut Vec<(usize, usize, usize, usize)>,
) {
    const SPLIT_ASPECT: f64 = 0.9; // w/h above which a split is attempted
    const MAX_DEPTH: u32 = 3;

    let mut stack = vec![(component, 0u32)];
    while let Some(((cx, cy, cw, ch), depth)) = stack.pop() {
        if depth >= MAX_DEPTH || (cw as f64) <= (ch as f64) * SPLIT_ASPECT {
            out.push((cx, cy, cw, ch));
            continue;
        }

        // Per-column foreground counts within the component bbox.
        let mut cols = vec![0u32; cw];
        for x in 0..cw {
            for y in 0..ch {
                if image[(cy + y) * width + (cx + x)] >= 128 {
                    cols[x] += 1;
                }
            }
        }

        // Search a band around the middle 60% of the width for the column
        // with the fewest foreground pixels.
        let lo = (cw as f64 * 0.2).round() as usize;
        let hi = cw.saturating_sub((cw as f64 * 0.2).round() as usize);
        let (lo, hi) = if lo + 1 < hi {
            (lo, hi)
        } else {
            (1, cw.saturating_sub(1).max(1))
        };

        let mut split_x = lo;
        let mut split_v = u32::MAX;
        for (x, &col) in cols.iter().enumerate().take(hi).skip(lo) {
            if col < split_v {
                split_v = col;
                split_x = x;
            }
        }

        // At this resolution even the widest single glyph stays narrower than
        // `ch * SPLIT_ASPECT`, so any component reaching here is a merged
        // multi-digit blob (e.g. a "6" whose curve touches the preceding "5").
        // We therefore always split at the lowest-density column; the min-size
        // guard below rejects bad slivers, and recognition score-gates halves.
        // Only bail if the "valley" is a completely solid column (no dip at
        // all), which would indicate a single thick stroke rather than a gap.
        if split_v >= ch as u32 {
            out.push((cx, cy, cw, ch));
            continue;
        }

        // Compute tight bounding boxes for each half.
        let left = tight_bbox(image, width, cx, cy, 0, split_x, ch);
        let right = tight_bbox(image, width, cx, cy, split_x, cw, ch);

        let s = scale.max(0.1);
        let min_w = ((3.0 * s).round() as usize).max(1);
        let min_h = ((10.0 * s).round() as usize).max(3);

        match (left, right) {
            (Some(l), Some(r)) if l.2 >= min_w && l.3 >= min_h && r.2 >= min_w && r.3 >= min_h => {
                stack.push((r, depth + 1));
                stack.push((l, depth + 1));
            }
            _ => out.push((cx, cy, cw, ch)),
        }
    }
}

/// Tighten a slice of a parent component to its non-empty bounding box.
/// `x0..x1` is in component-local coordinates; the returned bbox is global.
fn tight_bbox(
    image: &[u8],
    width: usize,
    cx: usize,
    cy: usize,
    x0: usize,
    x1: usize,
    ch: usize,
) -> Option<(usize, usize, usize, usize)> {
    let mut min_x = usize::MAX;
    let mut max_x = 0usize;
    let mut min_y = usize::MAX;
    let mut max_y = 0usize;
    let mut any = false;
    for y in 0..ch {
        for x in x0..x1 {
            if image[(cy + y) * width + (cx + x)] >= 128 {
                any = true;
                if cx + x < min_x {
                    min_x = cx + x;
                }
                if cx + x > max_x {
                    max_x = cx + x;
                }
                if cy + y < min_y {
                    min_y = cy + y;
                }
                if cy + y > max_y {
                    max_y = cy + y;
                }
            }
        }
    }
    if !any {
        return None;
    }
    Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

/// Aggregate statistics over enclosed background — the cells that a flood fill
/// from the image border cannot reach. Returns `(area_frac, cy_norm)` where
/// `area_frac` is the enclosed area as a fraction of the glyph and `cy_norm` is
/// the normalized vertical centroid (0 = top, 1 = bottom), or `None` if the
/// glyph has no enclosed loop at all.
fn enclosed_hole_stats(image: &[u8], width: usize, height: usize) -> Option<(f64, f64)> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut reachable = vec![false; width * height];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let push_bg = |x: usize, y: usize, reachable: &mut [bool], stack: &mut Vec<(usize, usize)>| {
        let idx = y * width + x;
        if image[idx] < 128 && !reachable[idx] {
            reachable[idx] = true;
            stack.push((x, y));
        }
    };
    for x in 0..width {
        push_bg(x, 0, &mut reachable, &mut stack);
        push_bg(x, height - 1, &mut reachable, &mut stack);
    }
    for y in 0..height {
        push_bg(0, y, &mut reachable, &mut stack);
        push_bg(width - 1, y, &mut reachable, &mut stack);
    }
    while let Some((x, y)) = stack.pop() {
        if x > 0 {
            push_bg(x - 1, y, &mut reachable, &mut stack);
        }
        if x + 1 < width {
            push_bg(x + 1, y, &mut reachable, &mut stack);
        }
        if y > 0 {
            push_bg(x, y - 1, &mut reachable, &mut stack);
        }
        if y + 1 < height {
            push_bg(x, y + 1, &mut reachable, &mut stack);
        }
    }
    let mut hole_cells = 0usize;
    let mut sum_y = 0usize;
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if image[idx] < 128 && !reachable[idx] {
                hole_cells += 1;
                sum_y += y;
            }
        }
    }
    if hole_cells == 0 {
        return None;
    }
    let area_frac = hole_cells as f64 / (width * height) as f64;
    let cy_norm = (sum_y as f64 / hole_cells as f64) / (height - 1).max(1) as f64;
    Some((area_frac, cy_norm))
}

/// Detect an enclosed background loop in the lower half of a glyph.
fn has_lower_loop(image: &[u8], width: usize, height: usize) -> bool {
    // Genuine 4: cy≈0.52, area_frac≈0.05. Genuine/misread 6: cy≈0.67, area_frac≈0.16.
    matches!(enclosed_hole_stats(image, width, height), Some((af, cy)) if cy >= 0.60 && af >= 0.08)
}

/// Whether the glyph has any meaningful enclosed loop (used to reject digits
/// that structurally require one, e.g. '9', when none is present).
fn has_enclosed_loop(image: &[u8], width: usize, height: usize) -> bool {
    matches!(enclosed_hole_stats(image, width, height), Some((af, _)) if af >= 0.04)
}

/// Recognize a single digit from an image region.
fn recognize_digit(image: &[u8], width: usize, height: usize, scale: f64) -> Option<char> {
    let mut best_digit = None;
    // Relax the threshold at low resolutions: bilinear downsampling and the
    // imperfect templates produce lower scores even on clean glyphs.
    let min_score = if scale < LOW_SCALE_CUTOFF {
        MIN_MATCH_SCORE_LOW
    } else {
        MIN_MATCH_SCORE
    };
    let mut best_score = min_score;
    // Track the best non-'9' candidate so we can fall back if a loop-less glyph
    // wins as '9' (see the 9-vs-2 disambiguation below).
    let mut best_non9_digit = None;
    let mut best_non9_score = min_score;

    for template in TEMPLATES {
        // Check aspect ratio compatibility
        let scaled_template_width = (template.width as f64 * scale) as usize;

        // Width ratio: penalize if image width is very different from expected template width
        let width_ratio = width as f64 / scaled_template_width.max(1) as f64;
        let width_penalty = if !(0.5..=2.0).contains(&width_ratio) {
            0.5 // Heavy penalty for very different widths
        } else if !(0.7..=1.4).contains(&width_ratio) {
            0.85 // Moderate penalty
        } else {
            1.0 // No penalty
        };

        let score = compute_match_score(image, width, height, template, scale) * width_penalty;
        if score > best_score {
            best_score = score;
            best_digit = Some(template.digit);
        }
        if template.digit != '9' && score > best_non9_score {
            best_non9_score = score;
            best_non9_digit = Some(template.digit);
        }
    }

    // Disambiguate 4 vs 6: the F1 metric favors '4' for low-res '6' glyphs, but
    // a '4' cannot have a large enclosed loop low in the glyph — only a '6' can.
    if best_digit == Some('4') && has_lower_loop(image, width, height) {
        return Some('6');
    }

    // Disambiguate 9 vs 2: an open-top '2' can out-score '2' as a '9' because
    // its upper arc mimics a 9's bowl. A real '9' always has an enclosed loop;
    // if there's none, drop '9' and take the best non-'9' candidate.
    if best_digit == Some('9') && !has_enclosed_loop(image, width, height) {
        return best_non9_digit;
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
    let binary: Vec<u8> = image
        .iter()
        .map(|&v| if v > 120 { 255 } else { 0 })
        .collect();

    // Find connected components (individual digits)
    let components = find_components(&binary, width, height, scale);

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
        let components = find_components(&image, 10, 10, 1.0);
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
        let components = find_components(&image, 20, 20, 1.0);
        assert_eq!(components.len(), 1);
    }

    fn art(rows: &[&str]) -> (Vec<u8>, usize, usize) {
        let h = rows.len();
        let w = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut buf = vec![0u8; w * h];
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if c == '#' {
                    buf[y * w + x] = 255;
                }
            }
        }
        (buf, w, h)
    }

    const GENUINE_4: &[&str] = &[
        "......#..",
        ".....##..",
        ".....##..",
        "....###..",
        "...##.#..",
        "..##..#..",
        "..##..#..",
        ".########",
        "#########",
        "......#..",
        "......#..",
    ];
    const MISREAD_6: &[&str] = &[
        "......###.",
        "......##..",
        ".....##...",
        "....##....",
        "...######.",
        "...##...##",
        "..##.....#",
        "####.....#",
        "##.#....##",
        "...###.###",
        "....#####.",
    ];
    const GENUINE_6: &[&str] = &[
        "....###.", "....##..", "...##...", "..##....", ".######.", ".##...##", "##.....#",
        "##.....#", ".#....##", ".###.###", "..#####.",
    ];

    #[test]
    fn lower_loop_distinguishes_4_from_6() {
        let (b4, w4, h4) = art(GENUINE_4);
        assert!(
            !has_lower_loop(&b4, w4, h4),
            "4 triangle is upper, not a loop"
        );
        let (b6, w6, h6) = art(MISREAD_6);
        assert!(has_lower_loop(&b6, w6, h6), "6 loop sits in the lower half");
        let (g6, gw6, gh6) = art(GENUINE_6);
        assert!(has_lower_loop(&g6, gw6, gh6), "genuine 6 loop detected");
    }

    #[test]
    fn recognize_digit_resolves_6_misread_as_4() {
        let (b4, w4, h4) = art(GENUINE_4);
        assert_eq!(recognize_digit(&b4, w4, h4, 0.5), Some('4'));
        let (b6, w6, h6) = art(MISREAD_6);
        assert_eq!(recognize_digit(&b6, w6, h6, 0.5), Some('6'));
    }

    // Open-top '2' (no enclosed loop) vs a genuine '9' (closed upper bowl).
    const OPEN_2: &[&str] = &[
        "....###..",
        "...######",
        "..###..##",
        "..##...##",
        "..##...##",
        ".......##",
        "......##.",
        ".....###.",
        "....###..",
        "...###...",
        "..#######",
        "#########",
    ];
    const GENUINE_9: &[&str] = &[
        "...##...", ".######.", ".##..###", "##....##", "##....##", "##....##", ".##..###",
        ".######.", "....###.", "...###..", "...##...", "..##....",
    ];

    #[test]
    fn enclosed_loop_distinguishes_9_from_open_2() {
        let (b2, w2, h2) = art(OPEN_2);
        assert!(
            !has_enclosed_loop(&b2, w2, h2),
            "open-top 2 has no enclosed loop"
        );
        let (b9, w9, h9) = art(GENUINE_9);
        assert!(
            has_enclosed_loop(&b9, w9, h9),
            "genuine 9 has a closed bowl"
        );
    }

    #[test]
    fn recognize_digit_rejects_9_without_loop() {
        // The open '2' must never be reported as '9'.
        let (b2, w2, h2) = art(OPEN_2);
        assert_ne!(recognize_digit(&b2, w2, h2, 0.5), Some('9'));
    }
}

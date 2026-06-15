//! Shared image preprocessing for the OCR recognizer (luma, autocontrast, polarity, canonical framing, upscaling) and name-row splitting/joining.

use crate::ocr::preprocess;

/// Extract a region from an RGB image.
pub(crate) fn extract_region(
    image: &[u8],
    img_width: usize,
    img_height: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut region = vec![0u8; width * height * 3];

    for dy in 0..height {
        for dx in 0..width {
            let src_x = x + dx;
            let src_y = y + dy;

            if src_x < img_width && src_y < img_height {
                let src_idx = (src_y * img_width + src_x) * 3;
                let dst_idx = (dy * width + dx) * 3;

                if src_idx + 2 < image.len() && dst_idx + 2 < region.len() {
                    region[dst_idx] = image[src_idx];
                    region[dst_idx + 1] = image[src_idx + 1];
                    region[dst_idx + 2] = image[src_idx + 2];
                }
            }
        }
    }

    region
}

/// Split a name buffer into separate row images when the game has wrapped the
/// name across multiple rows.
///
/// Returns one (image, width, height) tuple per detected text row. A single row
/// is returned unchanged (the whole buffer). Rows are only split apart on a
/// genuine blank gap — a run of consecutive rows with no text pixels that is
/// tall relative to the text itself — so the internal horizontal gaps inside a
/// glyph (e.g. between the strokes of a CJK character) never cause a false
/// split, and a normal single line is never cut through its x-height.
pub(crate) fn split_text_lines(
    image: &[u8],
    width: usize,
    height: usize,
) -> Vec<(Vec<u8>, usize, usize)> {
    let bands = detect_text_bands(image, width, height);

    // 0 or 1 band: not a wrapped name — return the whole buffer untouched so the
    // OCR engine sees the line with its original surrounding margin.
    if bands.len() <= 1 {
        return vec![(image.to_vec(), width, height)];
    }

    bands
        .iter()
        .map(|&(y_start, y_end)| extract_tight_line(image, width, y_start, y_end))
        .collect()
}

/// Find vertical text bands separated by genuine blank gaps.
///
/// A row is "text" if it contains any bright pixel (> 200; text is bright on a
/// dark background after autocontrast). Contiguous text rows form a raw band;
/// raw bands separated by a blank run shorter than `gap_min` are merged so that
/// intra-glyph gaps stay within a single band. `gap_min` scales with the
/// tallest band so the threshold adapts to the rendered text size.
fn detect_text_bands(image: &[u8], width: usize, height: usize) -> Vec<(usize, usize)> {
    let row_has_text: Vec<bool> = (0..height)
        .map(|y| (0..width).any(|x| image[y * width + x] > 200))
        .collect();

    let mut raw: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (y, &has_text) in row_has_text.iter().enumerate() {
        match (has_text, start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                raw.push((s, y));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        raw.push((s, height));
    }

    if raw.is_empty() {
        return raw;
    }

    let tallest = raw.iter().map(|&(s, e)| e - s).max().unwrap_or(0);
    let gap_min = (tallest / 6).max(8);

    let mut merged: Vec<(usize, usize)> = vec![raw[0]];
    for &(s, e) in &raw[1..] {
        let last = merged.last_mut().unwrap();
        if s - last.1 < gap_min {
            last.1 = e;
        } else {
            merged.push((s, e));
        }
    }
    merged
}

/// Tight-crop the text within a row band, with a small padding margin.
fn extract_tight_line(
    image: &[u8],
    width: usize,
    y_start: usize,
    y_end: usize,
) -> (Vec<u8>, usize, usize) {
    let mut line_min_y = y_end;
    let mut line_max_y = y_start;
    let mut line_min_x = width;
    let mut line_max_x = 0;

    for y in y_start..y_end {
        for x in 0..width {
            if image[y * width + x] > 200 {
                line_min_y = line_min_y.min(y);
                line_max_y = line_max_y.max(y);
                line_min_x = line_min_x.min(x);
                line_max_x = line_max_x.max(x);
            }
        }
    }

    if line_max_y < line_min_y || line_max_x < line_min_x {
        return (vec![0u8; 1], 1, 1);
    }

    let pad = 2;
    let crop_x = line_min_x.saturating_sub(pad);
    let crop_y = line_min_y.saturating_sub(pad);
    let crop_w = (line_max_x - line_min_x + 1 + pad * 2).min(width - crop_x);
    let crop_h = (line_max_y - line_min_y + 1 + pad * 2).min(y_end - crop_y);

    let mut cropped = vec![0u8; crop_w * crop_h];
    for y in 0..crop_h {
        for x in 0..crop_w {
            cropped[y * crop_w + x] = image[(crop_y + y) * width + crop_x + x];
        }
    }

    (cropped, crop_w, crop_h)
}

/// Lay wrapped name rows side by side into one logical line for OCR.
///
/// The game wraps a single name across rows; reassembling them horizontally
/// reconstructs the original line so the OCR engine reads it with proper line
/// context (a lone glyph per row is otherwise misread). Rows are placed
/// left-to-right on a dark canvas with a small inter-row gap and a quiet-zone
/// border, each vertically centred.
pub(crate) fn join_lines_horizontally(
    lines: &[(Vec<u8>, usize, usize)],
) -> (Vec<u8>, usize, usize) {
    const GAP: usize = 16;
    const PAD: usize = 30;

    let inner_h = lines.iter().map(|&(_, _, h)| h).max().unwrap_or(0);
    let inner_w: usize =
        lines.iter().map(|&(_, w, _)| w).sum::<usize>() + GAP * lines.len().saturating_sub(1);

    let canvas_w = inner_w + 2 * PAD;
    let canvas_h = inner_h + 2 * PAD;
    let mut canvas = vec![0u8; canvas_w * canvas_h];

    let mut x_off = PAD;
    for (line, lw, lh) in lines {
        let y_off = PAD + (inner_h - lh) / 2;
        for y in 0..*lh {
            for x in 0..*lw {
                canvas[(y_off + y) * canvas_w + (x_off + x)] = line[y * lw + x];
            }
        }
        x_off += lw + GAP;
    }

    (canvas, canvas_w, canvas_h)
}

/// Upscale strategy for the recognizer preprocessing.
enum Upscale {
    /// Continuous bilinear scale by `base / scale_factor` (resolution-driven).
    /// Used for the type and name fields.
    Continuous { base: f64, scale_factor: f64 },
    /// Integer factor toward a target per-line height (crop-driven). Used for
    /// the shard/timestamp strips, which stack `lines` text rows.
    LineHeight,
}

/// Vertical text-to-frame ratio every single-line recognizer crop is padded to.
/// The model is trained on this exact framing, so the training generator
/// (`training/generate_dataset.py`) MUST render at the same value — keep the two
/// in sync or the recognizer sees text at a scale it never learned.
const TEXT_FRAME_RATIO: f64 = 0.60;

/// Horizontal quiet zone added on each side of the text, as a fraction of the
/// text-band height. Small but non-zero: a tight crop removes the variable blank
/// margin (which the recognizer otherwise reads as a phantom edge glyph — e.g.
/// the doubled leading `O` in `OORCA`), and this constant adds back just enough
/// uniform breathing room. The generator renders the same quiet zone.
const QUIET_ZONE_RATIO: f64 = 0.15;

/// Floor for the quiet zone (px), so tiny crops still keep a 2px margin.
const MIN_QUIET_ZONE: usize = 2;

/// A row/column carries ink when its luma range (max − min) clears this; ~a
/// quarter of the full 0..=255 range that autocontrast stretches to. Used by
/// both the column trim and the canonical single-line framing, and polarity-
/// agnostic by construction (it keys off contrast, not absolute brightness).
const ACTIVITY_THRESHOLD: u8 = 64;

/// The only per-field difference left in the recognizer input: how the canonical
/// frame is scaled up toward a legible size. Everything else — luma, autocontrast,
/// polarity normalization, the tight-crop-then-pad framing — is identical across
/// type, name, shard, and timestamp so a single canonical preprocessing serves
/// every field (and, modulo a final polarity flip, either OCR backend).
pub(crate) struct PreprocessParams {
    upscale: Upscale,
}

impl PreprocessParams {
    /// Type/name banner line: scale continuously by `upscale_base / scale_factor`
    /// (2.0 for the type banner, 4.0 for the smaller name line).
    pub(crate) fn light_text(scale_factor: f64, upscale_base: f64) -> Self {
        Self {
            upscale: Upscale::Continuous {
                base: upscale_base,
                scale_factor,
            },
        }
    }

    /// Stockpile name line. Same canonical framing as every other field; named
    /// for call-site clarity.
    pub(crate) fn name(scale_factor: f64, upscale_base: f64) -> Self {
        Self::light_text(scale_factor, upscale_base)
    }

    /// Shard/timestamp strip: upscale toward a legible per-line height.
    pub(crate) fn strip() -> Self {
        Self {
            upscale: Upscale::LineHeight,
        }
    }
}

/// Shared preprocessing for every field read by the ocrs recognizer (type,
/// name, shard, timestamp). The step order is fixed; `params` selects the
/// optional polarity/padding steps and the upscale strategy so each field keeps
/// its established behavior behind a single code path.
pub(crate) fn preprocess_for_recognizer(
    image: &[u8],
    width: usize,
    height: usize,
    lines: usize,
    params: &PreprocessParams,
) -> (Vec<u8>, usize, usize) {
    // Standard luma conversion: 0.299*R + 0.587*G + 0.114*B.
    let mut processed = Vec::with_capacity(width * height);
    for chunk in image.chunks_exact(3) {
        let luma =
            ((77u16 * chunk[0] as u16 + 150u16 * chunk[1] as u16 + 29u16 * chunk[2] as u16 + 128)
                >> 8) as u8;
        processed.push(luma);
    }

    // Stretch contrast so text becomes legible regardless of base brightness;
    // low-contrast grey-on-grey names and bright info bars both normalize to the
    // full dynamic range.
    autocontrast(&mut processed, 2);

    // Normalize polarity to light-on-dark. The recognizer is trained light-on-
    // dark; an in-game theme can render any field dark-on-light, which decodes to
    // junk if fed uninverted. After autocontrast the background dominates one
    // extreme, so a bright mean means dark-text-on-light: flip it.
    let mean: u32 =
        processed.iter().map(|&v| v as u32).sum::<u32>() / processed.len().max(1) as u32;
    if mean > 127 {
        for v in processed.iter_mut() {
            *v = 255 - *v;
        }
    }

    // Canonical framing. A single line is tight-cropped to its ink bbox on both
    // axes and re-padded to TEXT_FRAME_RATIO with a quiet zone — the exact frame
    // the model is trained on, and free of the variable blank margin the
    // detection box carries (which the recognizer otherwise reads as a phantom
    // edge glyph). A multi-line strip keeps its stacked rows; only the horizontal
    // margin is tightened, since per-row vertical banding isn't meaningful across
    // stacked lines.
    let (processed, width, height) = if lines == 1 {
        fit_single_line_frame(&processed, width, height)
    } else {
        let band_h = (height / lines.max(1)).max(1);
        let qz = (((band_h as f64) * QUIET_ZONE_RATIO).round() as usize).max(MIN_QUIET_ZONE);
        let (cropped, new_w) = trim_columns_to_content(&processed, width, height, qz);
        (cropped, new_w, height)
    };

    apply_upscale(processed, width, height, lines, &params.upscale)
}

/// Scale the trimmed crop up per the chosen `Upscale` strategy.
fn apply_upscale(
    buf: Vec<u8>,
    width: usize,
    height: usize,
    lines: usize,
    upscale: &Upscale,
) -> (Vec<u8>, usize, usize) {
    match *upscale {
        Upscale::Continuous { base, scale_factor } => {
            let factor = base / scale_factor;
            let new_w = ((width as f64) * factor) as usize;
            let new_h = ((height as f64) * factor) as usize;
            let scaled = preprocess::upscale_bilinear(&buf, width, height, new_w, new_h);
            (scaled, new_w, new_h)
        }
        Upscale::LineHeight => {
            // Target a legible per-line height. At low resolutions a single line
            // can be ~13px tall, below what the model reads reliably; the factor
            // targets a per-line height rather than blindly multiplying, since
            // over-upscaling blurs and hurts OCR.
            const TARGET_LINE_HEIGHT: usize = 26;
            let line_height = (height / lines.max(1)).max(1);
            let factor = ((TARGET_LINE_HEIGHT + line_height / 2) / line_height).max(1);
            if factor > 1 {
                let new_w = width * factor;
                let new_h = height * factor;
                let scaled = preprocess::upscale_bilinear(&buf, width, height, new_w, new_h);
                (scaled, new_w, new_h)
            } else {
                (buf, width, height)
            }
        }
    }
}

/// Stretch the grayscale histogram to full [0, 255] range in place.
///
/// Mirrors PIL's `ImageOps.autocontrast`: `cutoff_percent` of the pixel
/// population is clipped from each end of the histogram before computing the
/// low/high bounds, so a few outlier pixels don't dominate the mapping.
/// Crop blank left/right margins of a single-line grayscale strip down to the
/// text extent (plus `margin` columns of breathing room on each side).
///
/// A column carrying text spans both ink and background pixels vertically, so
/// its luma range is large; a blank column is near-uniform. Using the per-column
/// range makes this polarity-agnostic, which matters because the UI theme can
/// render the strip as light-on-dark or dark-on-light. If no column clears the
/// activity threshold (a genuinely blank strip), the input is returned unchanged
/// rather than cropped to nothing.
fn trim_columns_to_content(
    gray: &[u8],
    width: usize,
    height: usize,
    margin: usize,
) -> (Vec<u8>, usize) {
    if width == 0 || height == 0 {
        return (gray.to_vec(), width);
    }

    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for x in 0..width {
        let (mut min, mut max) = (255u8, 0u8);
        for y in 0..height {
            let v = gray[y * width + x];
            min = min.min(v);
            max = max.max(v);
        }
        if max - min >= ACTIVITY_THRESHOLD {
            first.get_or_insert(x);
            last = x;
        }
    }

    let Some(first) = first else {
        return (gray.to_vec(), width);
    };

    let lo = first.saturating_sub(margin);
    let hi = (last + margin + 1).min(width);
    let new_w = hi - lo;

    let mut out = Vec::with_capacity(new_w * height);
    for y in 0..height {
        let row = y * width;
        out.extend_from_slice(&gray[row + lo..row + hi]);
    }
    (out, new_w)
}

/// Crop a single-line grayscale crop tight to its ink bounding box on both axes,
/// then re-pad to the canonical frame: a horizontal quiet zone of
/// [`QUIET_ZONE_RATIO`] × band-height on each side, and top/bottom padding so the
/// text band fills [`TEXT_FRAME_RATIO`] of the height. Background is dark (0)
/// after polarity normalization, so padding is dark.
///
/// This is the heart of the shared preprocessing: it strips the variable blank
/// margin the detection box carries (which the recognizer otherwise reads as a
/// phantom leading/trailing glyph — the doubled `O` in `OORCA`) and presents
/// every field at the one constant scale and framing the model is trained on.
///
/// Ink is detected by per-row / per-column luma *range* (max − min ≥
/// [`ACTIVITY_THRESHOLD`]), which is polarity-agnostic. A genuinely blank crop
/// (no row or column clears the threshold) is returned unchanged rather than
/// cropped to nothing.
///
/// The type region is cropped tight to its text slab upstream (in the detector),
/// so its crop is already text-only. The name region's layout varies
/// (pinned/unpinned/old-format) and can still carry a stray bright band, so the
/// vertical extent is taken from [`dominant_text_band`] — the brightest
/// contiguous stroke run — rather than a plain first..last span that a detached
/// band would inflate.
///
/// Fraction of the crop's peak brightness a pixel must reach to count as a text
/// stroke. Strokes are near-white after autocontrast + polarity normalization; a
/// dim noise gradient stays mid-grey and never clears this.
const INK_PIXEL_RATIO: f64 = 0.55;

/// Find the vertical [first, last] row span of the actual text line.
///
/// Assumes light-on-dark (the framing runs after polarity normalization). A row
/// belongs to the text when it carries bright *stroke* pixels (value ≥
/// [`INK_PIXEL_RATIO`] × the crop's peak); this is true even for ascender/cap
/// rows whose strokes are thin (low row mean) but bright, so they are kept — and
/// false for a dim noise gradient bleeding into the crop, which has no near-white
/// pixels and so forms a separate, non-inked gap. Among the contiguous inked runs
/// the one carrying the most stroke pixels wins, dropping a detached noise band;
/// the chosen run is then grown across any row that still carries a stroke pixel,
/// so thin cap/ascender tips and descender tails aren't clipped.
/// Returns `None` when no row carries ink (a blank crop).
fn dominant_text_band(gray: &[u8], width: usize, height: usize) -> Option<(usize, usize)> {
    if width == 0 || height == 0 {
        return None;
    }

    let peak = gray.iter().copied().max().unwrap_or(0);
    if peak == 0 {
        return None;
    }
    let ink_level = (peak as f64 * INK_PIXEL_RATIO).round() as u8;
    // A handful of bright pixels marks a real stroke row while ignoring isolated
    // sensor speckle; scales gently with width so it works at every resolution.
    let min_ink = (width / 128).max(2);

    let row_ink = |y: usize| -> usize {
        gray[y * width..y * width + width]
            .iter()
            .filter(|&&v| v >= ink_level)
            .count()
    };

    let mut best: Option<(usize, usize)> = None;
    let mut best_energy = 0usize;
    let mut run_start: Option<usize> = None;
    let mut run_energy = 0usize;
    for y in 0..=height {
        let ink = if y < height { row_ink(y) } else { 0 };
        if ink >= min_ink {
            run_start.get_or_insert(y);
            run_energy += ink;
        } else if let Some(start) = run_start.take() {
            if run_energy > best_energy {
                best_energy = run_energy;
                best = Some((start, y - 1));
            }
            run_energy = 0;
        }
    }

    // Grow the chosen run across any row that still carries a stroke pixel, so a
    // cap/ascender tip or descender tail (one or two bright pixels, below
    // `min_ink`) isn't clipped. The noise band has no near-white pixels, so this
    // stops at the blank gap before it rather than swallowing it.
    let (mut y0, mut y1) = best?;
    while y0 > 0 && row_ink(y0 - 1) >= 1 {
        y0 -= 1;
    }
    while y1 + 1 < height && row_ink(y1 + 1) >= 1 {
        y1 += 1;
    }
    Some((y0, y1))
}

fn fit_single_line_frame(gray: &[u8], width: usize, height: usize) -> (Vec<u8>, usize, usize) {
    if width == 0 || height == 0 {
        return (gray.to_vec(), width, height);
    }

    // Columns carrying ink (large vertical luma range).
    let mut x_first: Option<usize> = None;
    let mut x_last = 0usize;
    for x in 0..width {
        let (mut min, mut max) = (255u8, 0u8);
        for y in 0..height {
            let v = gray[y * width + x];
            min = min.min(v);
            max = max.max(v);
        }
        if max - min >= ACTIVITY_THRESHOLD {
            x_first.get_or_insert(x);
            x_last = x;
        }
    }

    // Vertical extent: the brightest contiguous stroke run, so a stray bright
    // band in a variable-layout name crop doesn't inflate the frame.
    let Some((y0, y_last)) = dominant_text_band(gray, width, height) else {
        return (gray.to_vec(), width, height);
    };
    let Some(x0) = x_first else {
        return (gray.to_vec(), width, height);
    };
    let band_w = x_last - x0 + 1;
    let band_h = y_last - y0 + 1;

    let quiet = (((band_h as f64) * QUIET_ZONE_RATIO).round() as usize).max(MIN_QUIET_ZONE);
    let desired_h = (((band_h as f64) / TEXT_FRAME_RATIO).round() as usize).max(band_h);
    let top = (desired_h - band_h) / 2;
    let new_w = band_w + 2 * quiet;

    let mut out = vec![0u8; new_w * desired_h];
    for ry in 0..band_h {
        let src = (y0 + ry) * width + x0;
        let dst = (top + ry) * new_w + quiet;
        out[dst..dst + band_w].copy_from_slice(&gray[src..src + band_w]);
    }
    (out, new_w, desired_h)
}

fn autocontrast(gray: &mut [u8], cutoff_percent: u32) {
    if gray.is_empty() {
        return;
    }

    let mut hist = [0u32; 256];
    for &v in gray.iter() {
        hist[v as usize] += 1;
    }

    let cut = (gray.len() as u32 * cutoff_percent) / 100;

    // Lowest value with population remaining after clipping `cut` from the bottom.
    let mut acc = 0u32;
    let mut lo = 0usize;
    for (v, &count) in hist.iter().enumerate() {
        acc += count;
        if acc > cut {
            lo = v;
            break;
        }
    }

    // Highest value with population remaining after clipping `cut` from the top.
    let mut acc = 0u32;
    let mut hi = 255usize;
    for (v, &count) in hist.iter().enumerate().rev() {
        acc += count;
        if acc > cut {
            hi = v;
            break;
        }
    }

    if hi <= lo {
        return; // Flat or inverted range — nothing to stretch.
    }

    let span = (hi - lo) as f32;
    for v in gray.iter_mut() {
        let clamped = (*v as usize).clamp(lo, hi);
        *v = (((clamped - lo) as f32 / span) * 255.0).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_region() {
        // Create a simple test image
        let mut image = vec![0u8; 10 * 10 * 3];
        // Set center pixel to red
        let center_idx = (5 * 10 + 5) * 3;
        image[center_idx] = 255;

        let region = extract_region(&image, 10, 10, 4, 4, 3, 3);

        // Region should be 3x3x3 = 27 bytes
        assert_eq!(region.len(), 27);

        // Check that the red pixel is captured
        let region_center = (3 + 1) * 3;
        assert_eq!(region[region_center], 255);
    }

    #[test]
    fn autocontrast_stretches_to_full_range() {
        // A low-contrast band [100, 140] should stretch to span [0, 255].
        let mut gray: Vec<u8> = (0..1000)
            .map(|i| 100 + (i % 41) as u8) // values in [100, 140]
            .collect();
        autocontrast(&mut gray, 0);
        assert_eq!(*gray.iter().min().unwrap(), 0);
        assert_eq!(*gray.iter().max().unwrap(), 255);
    }

    #[test]
    fn autocontrast_handles_flat_input() {
        // A uniform image has hi == lo; it must be left untouched, not divided by zero.
        let mut gray = vec![128u8; 256];
        autocontrast(&mut gray, 2);
        assert!(gray.iter().all(|&v| v == 128));
    }

    #[test]
    fn autocontrast_ignores_empty() {
        let mut gray: Vec<u8> = Vec::new();
        autocontrast(&mut gray, 2); // must not panic
        assert!(gray.is_empty());
    }

    #[test]
    fn trim_columns_crops_blank_margins_to_text_extent() {
        // 10-wide, 4-tall strip: a high-contrast column at x=3 and x=4 (text),
        // everything else flat (blank). With margin 1 the kept span is [2, 6).
        let width = 10;
        let height = 4;
        let mut gray = vec![0u8; width * height];
        // Text columns carry ink on some rows and background on others, giving
        // them a large vertical range; a fully-uniform column would not count.
        for &x in &[3usize, 4] {
            gray[width + x] = 255; // row 1
            gray[2 * width + x] = 255; // row 2
        }
        let (out, new_w) = trim_columns_to_content(&gray, width, height, 1);
        assert_eq!(new_w, 4); // cols 2,3,4,5
        assert_eq!(out.len(), new_w * height);
    }

    #[test]
    fn trim_columns_leaves_blank_strip_unchanged() {
        // No column clears the activity threshold: return the input untouched
        // rather than cropping to nothing.
        let gray = vec![40u8; 8 * 3];
        let (out, new_w) = trim_columns_to_content(&gray, 8, 3, 2);
        assert_eq!(new_w, 8);
        assert_eq!(out, gray);
    }

    #[test]
    fn fit_frame_crops_to_ink_and_pads_to_ratio() {
        // 12-wide, 12-tall crop with a high-contrast 2x2 ink block at rows 5..=6,
        // cols 4..=5 (band 2x2). Expect: tight to the 2x2, quiet zone =
        // max(2, round(2*0.15)) = 2 px each side -> width 2 + 4 = 6; vertical pad
        // to round(2 / 0.60) = 3 rows tall, band centred (1 row top pad).
        let width = 12;
        let height = 12;
        let mut gray = vec![0u8; width * height];
        for y in 5..=6 {
            for x in 4..=5 {
                gray[y * width + x] = 255;
            }
        }
        let (out, new_w, new_h) = fit_single_line_frame(&gray, width, height);
        assert_eq!(
            (new_w, new_h),
            (6, 3),
            "tight crop + quiet zone + ratio pad"
        );
        assert_eq!(out.len(), new_w * new_h);
        // desired_h = round(2/0.60) = 3, top pad = (3-2)/2 = 0: the single pad row
        // lands at the bottom. Ink sits inside the 2px quiet zone (cols 2..=3).
        assert!(
            out[2 * new_w..].iter().all(|&v| v == 0),
            "bottom row is dark padding"
        );
        assert_eq!(
            out[2], 255,
            "ink starts inside the quiet zone on the first row"
        );
    }

    #[test]
    fn fit_frame_leaves_blank_crop_unchanged() {
        // No column/row clears the activity threshold: return the input untouched
        // rather than cropping to nothing.
        let gray = vec![20u8; 8 * 6];
        let (out, new_w, new_h) = fit_single_line_frame(&gray, 8, 6);
        assert_eq!((new_w, new_h), (8, 6));
        assert_eq!(out, gray);
    }

    #[test]
    fn presets_share_canonical_framing_and_differ_only_in_upscale() {
        // Every field flows through the one canonical preprocessing; the only
        // per-field knob left is the upscale strategy.
        match PreprocessParams::light_text(0.5, 4.0).upscale {
            Upscale::Continuous { base, scale_factor } => {
                assert_eq!((base, scale_factor), (4.0, 0.5));
            }
            Upscale::LineHeight => panic!("light_text must use a continuous upscale"),
        }
        // name is just a clarity alias for light_text.
        assert!(matches!(
            PreprocessParams::name(0.5, 4.0).upscale,
            Upscale::Continuous { .. }
        ));
        assert!(matches!(
            PreprocessParams::strip().upscale,
            Upscale::LineHeight
        ));
    }

    /// Build a grayscale buffer where the given inclusive row ranges are "text"
    /// (a single bright pixel per row) and everything else is background.
    fn buffer_with_text_rows(width: usize, height: usize, rows: &[(usize, usize)]) -> Vec<u8> {
        let mut buf = vec![0u8; width * height];
        for &(start, end) in rows {
            for y in start..=end {
                buf[y * width] = 255; // one bright pixel marks the row as text
            }
        }
        buf
    }

    #[test]
    fn detect_text_bands_merges_single_block() {
        // One continuous block of text -> exactly one band.
        let buf = buffer_with_text_rows(10, 100, &[(20, 80)]);
        let bands = detect_text_bands(&buf, 10, 100);
        assert_eq!(bands.len(), 1);
    }

    #[test]
    fn detect_text_bands_splits_on_tall_gap() {
        // Two blocks separated by a tall blank gap -> two bands (wrapped name).
        let buf = buffer_with_text_rows(10, 120, &[(0, 30), (70, 100)]);
        let bands = detect_text_bands(&buf, 10, 120);
        assert_eq!(bands.len(), 2);
    }

    #[test]
    fn detect_text_bands_ignores_small_intra_glyph_gap() {
        // A short blank run inside a glyph must not split the band.
        let buf = buffer_with_text_rows(10, 100, &[(20, 50), (53, 80)]); // 2-row gap
        let bands = detect_text_bands(&buf, 10, 100);
        assert_eq!(bands.len(), 1);
    }

    #[test]
    fn split_text_lines_returns_whole_buffer_for_single_row() {
        let buf = buffer_with_text_rows(10, 100, &[(20, 80)]);
        let lines = split_text_lines(&buf, 10, 100);
        assert_eq!(lines.len(), 1);
        assert_eq!((lines[0].1, lines[0].2), (10, 100)); // unchanged dimensions
    }

    #[test]
    fn join_lines_horizontally_places_rows_side_by_side() {
        // Two 4x4 white tiles -> width grows, height is single row + padding.
        let a = (vec![255u8; 16], 4, 4);
        let b = (vec![255u8; 16], 4, 4);
        let (canvas, w, h) = join_lines_horizontally(&[a, b]);
        assert_eq!(w, 4 + 4 + 16 + 2 * 30); // widths + GAP + 2*PAD
        assert_eq!(h, 4 + 2 * 30); // tallest row + 2*PAD
        assert_eq!(canvas.len(), w * h);
        // The quiet-zone border stays background (dark).
        assert_eq!(canvas[0], 0);
    }

    #[test]
    fn dominant_text_band_finds_single_contiguous_run() {
        // One bright text run on rows 6..=12 (2 stroke pixels per row, clearing
        // min_ink=2 at this width); everything else dark. The band is that run.
        let (w, h) = (16usize, 24usize);
        let mut g = vec![0u8; w * h];
        for y in 6..=12 {
            g[y * w] = 255;
            g[y * w + 1] = 255;
        }
        assert_eq!(dominant_text_band(&g, w, h), Some((6, 12)));
    }

    #[test]
    fn dominant_text_band_keeps_brightest_run_and_drops_dim_noise() {
        // A dim noise band (value 100, below the 0.55*255≈140 ink level) on top,
        // a short bright run, and a taller bright run lower down. The tall run
        // carries the most stroke pixels, so it wins; the noise never counts.
        let (w, h) = (16usize, 30usize);
        let mut g = vec![0u8; w * h];
        for y in 0..=4 {
            for x in 0..w {
                g[y * w + x] = 100; // dim noise strip — no near-white pixels
            }
        }
        for y in 8..=9 {
            g[y * w] = 255;
            g[y * w + 1] = 255; // small bright run (energy 4)
        }
        for y in 18..=26 {
            g[y * w] = 255;
            g[y * w + 1] = 255; // tall bright run (energy 18) — wins
        }
        assert_eq!(dominant_text_band(&g, w, h), Some((18, 26)));
    }

    #[test]
    fn dominant_text_band_grows_across_thin_cap_and_descender_tips() {
        // Main run rows 10..=15 (2 px each). A single bright pixel one row above
        // (a cap/ascender tip) and one row below (a descender tail) fall below
        // min_ink but still carry a stroke pixel, so the band grows to include
        // them rather than clipping the glyph.
        let (w, h) = (16usize, 24usize);
        let mut g = vec![0u8; w * h];
        for y in 10..=15 {
            g[y * w] = 255;
            g[y * w + 1] = 255;
        }
        g[9 * w] = 255; // cap tip above
        g[16 * w] = 255; // descender below
        assert_eq!(dominant_text_band(&g, w, h), Some((9, 16)));
    }

    #[test]
    fn dominant_text_band_ignores_sub_min_ink_speckle() {
        // One bright pixel per row (below min_ink=2) is isolated sensor speckle,
        // not a stroke row: no run is ever seeded, so there is no band.
        let (w, h) = (16usize, 20usize);
        let mut g = vec![0u8; w * h];
        for y in 0..h {
            g[y * w] = 255;
        }
        assert_eq!(dominant_text_band(&g, w, h), None);
    }

    #[test]
    fn dominant_text_band_returns_none_for_blank_or_empty() {
        // All-dark crop has no peak; zero-sized crop has no rows.
        assert_eq!(dominant_text_band(&[0u8; 16 * 4], 16, 4), None);
        assert_eq!(dominant_text_band(&[], 0, 0), None);
    }
}

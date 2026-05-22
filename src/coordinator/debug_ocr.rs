//! Optional OCR debug image dumps, gated on the `FS_DEBUG_OCR` environment
//! variable. The feature is inert unless `FS_DEBUG_OCR=1`, so normal users
//! never see any output.
//!
//! When enabled, the scanner writes (to the current working directory):
//!   - `debug_image.png` — the source screenshot with a coloured contour around
//!     every detected region (stockpile type, name, shard/timestamp, icon
//!     regions, and quantity boxes).
//!   - `debug_<label>.png` — each individual buffer handed to OCR: the type,
//!     timestamp, and shard crops, plus the name crop. When the name wraps
//!     across two rows, every detected line is saved (`debug_name_line0`,
//!     `debug_name_line1`, …) alongside the merged single-line image
//!     (`debug_name_merged`).
//!
//! Saving failures are deliberately ignored: debugging output must never affect
//! a real scan.

use crate::detector::DetectedRegions;

/// Whether debug dumps are enabled (`FS_DEBUG_OCR=1`).
pub fn enabled() -> bool {
    std::env::var("FS_DEBUG_OCR")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Save a single-channel (grayscale) OCR buffer as `debug_<label>.png`.
pub fn save_gray(label: &str, buf: &[u8], width: usize, height: usize) {
    if width == 0 || height == 0 || buf.len() < width * height {
        return;
    }
    if let Some(img) =
        image::GrayImage::from_raw(width as u32, height as u32, buf[..width * height].to_vec())
    {
        let _ = img.save(format!("debug_{label}.png"));
    }
}

/// Save the source image (`debug_image.png`) with a coloured contour around
/// every detected region. Expects packed RGB (`channels = 3`) input.
pub fn save_regions_overlay(image: &[u8], width: usize, height: usize, regions: &DetectedRegions) {
    if width == 0 || height == 0 || image.len() < width * height * 3 {
        return;
    }

    let mut canvas = image[..width * height * 3].to_vec();

    // Quantity boxes (green) and their derived icon regions (blue).
    let green = [0u8, 255, 0];
    let blue = [0u8, 128, 255];
    for &(bx, by) in &regions.quantity_boxes {
        draw_rect(
            &mut canvas,
            width,
            height,
            (bx, by, regions.box_width, regions.box_height),
            green,
        );
    }
    for &rect in &regions.icon_regions {
        draw_rect(&mut canvas, width, height, rect, blue);
    }

    // Metadata regions, each in a distinct colour.
    let metadata = [
        (regions.type_region, [255u8, 0, 0]),    // type  -> red
        (regions.name_region, [255u8, 255, 0]),  // name  -> yellow
        (regions.shard_region, [0u8, 255, 255]), // shard -> cyan
    ];
    for (region, colour) in metadata {
        if let Some(rect) = region {
            draw_rect(&mut canvas, width, height, rect, colour);
        }
    }

    if let Some(img) = image::RgbImage::from_raw(width as u32, height as u32, canvas) {
        let _ = img.save("debug_image.png");
    }
}

/// Draw a hollow rectangle (2px border) onto a packed-RGB canvas, clipped to
/// bounds. Negative or out-of-range coordinates are skipped per-pixel.
fn draw_rect(
    canvas: &mut [u8],
    width: usize,
    height: usize,
    rect: (i32, i32, i32, i32),
    colour: [u8; 3],
) {
    let (x, y, w, h) = rect;
    if w <= 0 || h <= 0 {
        return;
    }
    const THICKNESS: i32 = 2;
    let x0 = x;
    let y0 = y;
    let x1 = x + w - 1;
    let y1 = y + h - 1;

    let mut put = |px: i32, py: i32| {
        if px < 0 || py < 0 || px as usize >= width || py as usize >= height {
            return;
        }
        let idx = (py as usize * width + px as usize) * 3;
        canvas[idx] = colour[0];
        canvas[idx + 1] = colour[1];
        canvas[idx + 2] = colour[2];
    };

    for t in 0..THICKNESS {
        // Top and bottom edges.
        for px in x0..=x1 {
            put(px, y0 + t);
            put(px, y1 - t);
        }
        // Left and right edges.
        for py in y0..=y1 {
            put(x0 + t, py);
            put(x1 - t, py);
        }
    }
}

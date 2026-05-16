//! Geometry types for stockpile detection.

use serde::{Deserialize, Serialize};

/// Coordinate pair (x, y).
pub type Coordinates = (i32, i32);

/// Bounding rectangle (x, y, width, height).
pub type BoundingRect = (i32, i32, i32, i32);

/// Information about a detected item group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    /// Number of items in this group.
    pub size: usize,
    /// Starting index of this group in the items list.
    pub start_index: usize,
}

impl GroupInfo {
    /// Create a new group info.
    pub fn new(size: usize, start_index: usize) -> Self {
        Self { size, start_index }
    }
}

/// All detected regions from a stockpile screenshot.
#[derive(Debug, Clone)]
pub struct DetectedRegions {
    /// Coordinates of quantity boxes.
    pub quantity_boxes: Vec<Coordinates>,
    /// Coordinates of icon regions (derived from quantity boxes).
    pub icon_regions: Vec<BoundingRect>,
    /// Group information for item organization.
    pub groups: Vec<GroupInfo>,
    /// Scale factor relative to base resolution.
    pub scale_factor: f64,
    /// Vertical resolution of the image.
    pub vertical_resolution: i32,
    /// Whether the first row has only a single item.
    pub has_single_item_first_row: bool,
    /// Region for stockpile type text.
    pub type_region: Option<BoundingRect>,
    /// Region for stockpile name text.
    pub name_region: Option<BoundingRect>,
    /// Region for shard/timestamp text.
    pub shard_region: Option<BoundingRect>,
    /// Scaled box width for quantity regions.
    pub box_width: i32,
    /// Scaled box height for quantity regions.
    pub box_height: i32,
    /// Info bar height (first_box_y - roi_y), used to determine stockpile format.
    pub info_bar_height: i32,
}

impl DetectedRegions {
    /// Create empty detected regions.
    pub fn new(
        scale_factor: f64,
        vertical_resolution: i32,
        box_width: i32,
        box_height: i32,
    ) -> Self {
        Self {
            quantity_boxes: Vec::new(),
            icon_regions: Vec::new(),
            groups: Vec::new(),
            scale_factor,
            vertical_resolution,
            has_single_item_first_row: false,
            type_region: None,
            name_region: None,
            shard_region: None,
            box_width,
            box_height,
            info_bar_height: 0,
        }
    }

    /// Get the total number of detected items.
    pub fn item_count(&self) -> usize {
        self.quantity_boxes.len()
    }

    /// Check if any items were detected.
    pub fn is_empty(&self) -> bool {
        self.quantity_boxes.is_empty()
    }
}

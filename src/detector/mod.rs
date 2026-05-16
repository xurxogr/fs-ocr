//! Stockpile detection components.

mod black_box;
mod geometry;
mod grey_mask;

pub use black_box::{BlackBoxDetector, BlackBoxResult};
pub use geometry::{BoundingRect, Coordinates, DetectedRegions, GroupInfo};
pub use grey_mask::GreyMaskDetector;

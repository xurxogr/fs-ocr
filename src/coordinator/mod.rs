//! Pipeline orchestration for stockpile scanning.

mod debug_ocr;
mod pipeline;
mod validation;

pub use pipeline::ScanPipeline;
pub use validation::validate_descending_order;

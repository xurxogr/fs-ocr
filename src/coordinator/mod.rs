//! Pipeline orchestration for stockpile scanning.

mod debug_ocr;
mod metadata_parse;
mod pipeline;
mod region_preprocess;
mod validation;

pub use pipeline::ScanPipeline;
pub use validation::validate_descending_order;

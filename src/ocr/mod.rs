//! OCR components for text and quantity extraction.

pub mod quantity;
pub mod tesseract;

pub use quantity::parse_quantity;
pub use tesseract::TextExtractor;

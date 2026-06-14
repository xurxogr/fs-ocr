//! Template matching components.

pub mod database;
pub mod label_match;
pub mod matching;
pub mod phash;
pub mod public_match;
pub mod type_match;

pub use database::{IconTemplate, TemplateDatabase};
pub use matching::{MatchResult, TemplateMatcher};
pub use phash::compute_phash;

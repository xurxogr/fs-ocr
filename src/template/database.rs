//! HDF5 template database loading and management.
//!
//! The HDF5 database format stores pre-rendered icon templates for template matching.
//! Each resolution group contains:
//! - `images`: (N, H, W, 3) uint8 array of RGB images
//! - `codes`: Variable-length UTF-8 strings (item codes)
//! - `mods`: Variable-length UTF-8 strings (mod names)
//! - `crated`: Boolean array
//! - `faction`: uint8 array (indices into ItemFaction enum)
//! - `category`: uint8 array (indices into ItemCategory enum)
//! - `phash`: uint64 array (perceptual hashes)
//!
//! HDF5 file structure:
//! ```text
//! database.hdf5
//! ├── Attributes
//! │   ├── version: 2
//! │   ├── format: "hdf5"
//! │   └── resolutions: ["664", "720", "1080", ...]
//! │
//! └── /{resolution}  (e.g., /1080)
//!     ├── Attributes
//!     │   ├── resolution, template_count, icon_size, version
//!     └── Datasets
//!         ├── images, codes, mods, crated, faction, category, phash
//! ```

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::constants::find_closest_resolution;
use crate::enums::{ItemCategory, ItemFaction};
use crate::error::{FsOcrError, Result};

/// Current database version.
pub const DATABASE_VERSION: u32 = 2;

/// A single icon template from the database.
#[derive(Debug, Clone)]
pub struct IconTemplate {
    /// Pre-rendered icon image data (H x W x 3, RGB, row-major).
    pub image_data: Vec<u8>,
    /// Item code identifier.
    pub code: String,
    /// Mod name (e.g., "vanilla", "airborne").
    pub mod_name: String,
    /// Item faction.
    pub faction: ItemFaction,
    /// Item category.
    pub category: ItemCategory,
    /// Whether this is a crated item.
    pub crated: bool,
    /// Perceptual hash for fast filtering.
    pub phash: u64,
}

/// Builder for creating IconTemplate instances.
#[derive(Debug, Default)]
pub struct IconTemplateBuilder {
    image_data: Vec<u8>,
    code: String,
    mod_name: String,
    faction: ItemFaction,
    category: ItemCategory,
    crated: bool,
    phash: u64,
}

impl IconTemplateBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the image data.
    pub fn image(mut self, data: Vec<u8>) -> Self {
        self.image_data = data;
        self
    }

    /// Set the item code.
    pub fn code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }

    /// Set the mod name.
    pub fn mod_name(mut self, mod_name: impl Into<String>) -> Self {
        self.mod_name = mod_name.into();
        self
    }

    /// Set the faction.
    pub fn faction(mut self, faction: ItemFaction) -> Self {
        self.faction = faction;
        self
    }

    /// Set the category.
    pub fn category(mut self, category: ItemCategory) -> Self {
        self.category = category;
        self
    }

    /// Set whether the item is crated.
    pub fn crated(mut self, crated: bool) -> Self {
        self.crated = crated;
        self
    }

    /// Set the perceptual hash.
    pub fn phash(mut self, phash: u64) -> Self {
        self.phash = phash;
        self
    }

    /// Build the IconTemplate.
    pub fn build(self) -> IconTemplate {
        IconTemplate {
            image_data: self.image_data,
            code: self.code,
            mod_name: self.mod_name,
            faction: self.faction,
            category: self.category,
            crated: self.crated,
            phash: self.phash,
        }
    }
}

impl IconTemplate {
    /// Create a new builder for IconTemplate.
    pub fn builder() -> IconTemplateBuilder {
        IconTemplateBuilder::new()
    }
}

/// Template database for a specific resolution.
#[derive(Debug)]
pub struct TemplateDatabase {
    /// Resolution this database is for.
    pub resolution: i32,
    /// All templates at this resolution.
    pub templates: Vec<IconTemplate>,
    /// Lookup by faction.
    pub faction_lookup: HashMap<ItemFaction, HashSet<usize>>,
    /// Lookup by category.
    pub category_lookup: HashMap<ItemCategory, HashSet<usize>>,
    /// Lookup by mod.
    pub mod_lookup: HashMap<String, HashSet<usize>>,
    /// All pHashes as a contiguous array for vectorized operations.
    pub phash_array: Vec<u64>,
    // === Precomputed NCC statistics (computed on load for fast matching) ===
    /// Mean of each template's pixels.
    pub ncc_means: Vec<f32>,
    /// Inverse standard deviation (1/std) for each template.
    pub ncc_inv_stds: Vec<f32>,
}

impl TemplateDatabase {
    /// Create an empty template database.
    pub fn new(resolution: i32) -> Self {
        Self {
            resolution,
            templates: Vec::new(),
            faction_lookup: HashMap::new(),
            category_lookup: HashMap::new(),
            mod_lookup: HashMap::new(),
            phash_array: Vec::new(),
            ncc_means: Vec::new(),
            ncc_inv_stds: Vec::new(),
        }
    }

    /// Compute NCC statistics (mean, inv_std) for a template image.
    #[inline]
    fn compute_ncc_stats(image_data: &[u8]) -> (f32, f32) {
        if image_data.is_empty() {
            return (0.0, 0.0);
        }
        let n = image_data.len() as f32;
        let sum: u64 = image_data.iter().map(|&x| x as u64).sum();
        let mean = sum as f32 / n;

        let var_sum: f32 = image_data
            .iter()
            .map(|&x| {
                let diff = x as f32 - mean;
                diff * diff
            })
            .sum();

        let std = var_sum.sqrt();
        let inv_std = if std > 1e-6 { 1.0 / std } else { 0.0 };

        (mean, inv_std)
    }

    /// Load a template database from an HDF5 file.
    ///
    /// Loads templates for the closest available resolution from the HDF5 database.
    pub fn load<P: AsRef<Path>>(path: P, resolution: i32) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(FsOcrError::Database(format!(
                "Database file not found: {}",
                path.display()
            )));
        }

        // Find closest supported resolution
        let closest = find_closest_resolution(resolution);

        // Open HDF5 file
        let file = hdf5::File::open(path)
            .map_err(|e| FsOcrError::Database(format!("Failed to open HDF5 file: {}", e)))?;

        // Try to open the resolution group
        let group_name = closest.to_string();
        let group = file.group(&group_name).map_err(|e| {
            FsOcrError::Database(format!(
                "Resolution group '{}' not found in database: {}",
                group_name, e
            ))
        })?;

        // Verify version
        let version: u32 = group
            .attr("version")
            .and_then(|a| a.read_scalar())
            .unwrap_or(0);

        if version != DATABASE_VERSION {
            return Err(FsOcrError::Database(format!(
                "Database version mismatch: got {}, expected {}. Regenerate with 'fs generate-templates'.",
                version, DATABASE_VERSION
            )));
        }

        // Get template count
        let template_count: usize = group
            .attr("template_count")
            .and_then(|a| a.read_scalar::<i64>())
            .map(|v| v as usize)
            .unwrap_or(0);

        if template_count == 0 {
            return Ok(Self::new(closest));
        }

        let mut db = Self::new(closest);

        // Load datasets
        let images_ds = group
            .dataset("images")
            .map_err(|e| FsOcrError::Database(format!("Failed to load images dataset: {}", e)))?;
        let codes_ds = group
            .dataset("codes")
            .map_err(|e| FsOcrError::Database(format!("Failed to load codes dataset: {}", e)))?;
        let mods_ds = group
            .dataset("mods")
            .map_err(|e| FsOcrError::Database(format!("Failed to load mods dataset: {}", e)))?;
        let crated_ds = group
            .dataset("crated")
            .map_err(|e| FsOcrError::Database(format!("Failed to load crated dataset: {}", e)))?;
        let faction_ds = group
            .dataset("faction")
            .map_err(|e| FsOcrError::Database(format!("Failed to load faction dataset: {}", e)))?;
        let category_ds = group
            .dataset("category")
            .map_err(|e| FsOcrError::Database(format!("Failed to load category dataset: {}", e)))?;
        let phash_ds = group
            .dataset("phash")
            .map_err(|e| FsOcrError::Database(format!("Failed to load phash dataset: {}", e)))?;

        // Get image dimensions from dataset shape
        let images_shape = images_ds.shape();
        let img_h = images_shape[1] as i32;
        let img_w = images_shape[2] as i32;
        let img_c = images_shape[3];
        let pixels_per_image = (img_h as usize) * (img_w as usize) * img_c;

        // Read all data as flat vectors
        let images_flat: Vec<u8> = images_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read images: {}", e)))?;
        let codes: Vec<hdf5::types::VarLenUnicode> = codes_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read codes: {}", e)))?;
        let mods: Vec<hdf5::types::VarLenUnicode> = mods_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read mods: {}", e)))?;
        let crated: Vec<bool> = crated_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read crated: {}", e)))?;
        let faction_indices: Vec<u8> = faction_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read faction: {}", e)))?;
        let category_indices: Vec<u8> = category_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read category: {}", e)))?;
        let phashes: Vec<u64> = phash_ds
            .read_raw()
            .map_err(|e| FsOcrError::Database(format!("Failed to read phash: {}", e)))?;

        // Create templates
        for i in 0..template_count {
            // Extract image data for this template from flat array
            let start = i * pixels_per_image;
            let end = start + pixels_per_image;
            let image_data: Vec<u8> = images_flat[start..end].to_vec();

            let code = codes[i].to_string();
            let mod_name = mods[i].to_string();
            let is_crated = crated[i];
            let faction: ItemFaction = faction_indices[i].into();
            let category: ItemCategory = category_indices[i].into();
            let phash = phashes[i];

            let template = IconTemplate::builder()
                .image(image_data)
                .code(code)
                .mod_name(mod_name)
                .faction(faction)
                .category(category)
                .crated(is_crated)
                .phash(phash)
                .build();

            db.add_template(template);
        }

        db.rebuild_lookups();
        Ok(db)
    }

    /// Add a template to the database.
    pub fn add_template(&mut self, template: IconTemplate) {
        let idx = self.templates.len();

        // Update lookups
        self.faction_lookup
            .entry(template.faction)
            .or_default()
            .insert(idx);
        self.category_lookup
            .entry(template.category)
            .or_default()
            .insert(idx);
        self.mod_lookup
            .entry(template.mod_name.clone())
            .or_default()
            .insert(idx);
        self.phash_array.push(template.phash);

        // Compute and store NCC statistics
        let (mean, inv_std) = Self::compute_ncc_stats(&template.image_data);
        self.ncc_means.push(mean);
        self.ncc_inv_stds.push(inv_std);

        self.templates.push(template);
    }

    /// Get candidates matching the given filters.
    pub fn get_candidates(
        &self,
        faction: Option<ItemFaction>,
        mod_name: Option<&str>,
        category: Option<ItemCategory>,
        crated: Option<bool>,
        excluded_codes: Option<&HashSet<String>>,
    ) -> Vec<usize> {
        let mut candidates: HashSet<usize> = (0..self.templates.len()).collect();

        // Filter by category
        if let Some(cat) = category {
            if cat != ItemCategory::Invalid {
                if let Some(indices) = self.category_lookup.get(&cat) {
                    candidates = candidates.intersection(indices).copied().collect();
                }
            }
        }

        // Filter by mod
        if let Some(mod_str) = mod_name {
            if let Some(indices) = self.mod_lookup.get(mod_str) {
                candidates = candidates.intersection(indices).copied().collect();
            }
        }

        // Filter by faction (include neutral items)
        if let Some(fac) = faction {
            if fac != ItemFaction::Neutral {
                let mut faction_candidates = HashSet::new();
                if let Some(indices) = self.faction_lookup.get(&fac) {
                    faction_candidates.extend(indices);
                }
                if let Some(indices) = self.faction_lookup.get(&ItemFaction::Neutral) {
                    faction_candidates.extend(indices);
                }
                candidates = candidates
                    .intersection(&faction_candidates)
                    .copied()
                    .collect();
            }
        }

        // Filter by crated status
        if let Some(is_crated) = crated {
            candidates.retain(|&i| self.templates[i].crated == is_crated);
        }

        // Exclude specific codes
        if let Some(excluded) = excluded_codes {
            candidates.retain(|&i| !excluded.contains(&self.templates[i].code));
        }

        // Sort by index so candidate order is deterministic across calls.
        // HashSet iteration order varies per call (random seed), which would
        // otherwise make tiebreaks between equal-scoring templates nondeterministic.
        let mut candidates: Vec<usize> = candidates.into_iter().collect();
        candidates.sort_unstable();
        candidates
    }

    /// Rebuild lookup tables after adding templates.
    pub fn rebuild_lookups(&mut self) {
        self.faction_lookup.clear();
        self.category_lookup.clear();
        self.mod_lookup.clear();
        self.phash_array.clear();
        self.ncc_means.clear();
        self.ncc_inv_stds.clear();

        for (i, template) in self.templates.iter().enumerate() {
            self.faction_lookup
                .entry(template.faction)
                .or_default()
                .insert(i);
            self.category_lookup
                .entry(template.category)
                .or_default()
                .insert(i);
            self.mod_lookup
                .entry(template.mod_name.clone())
                .or_default()
                .insert(i);
            self.phash_array.push(template.phash);

            // Compute NCC statistics
            let (mean, inv_std) = Self::compute_ncc_stats(&template.image_data);
            self.ncc_means.push(mean);
            self.ncc_inv_stds.push(inv_std);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_candidates_empty() {
        let db = TemplateDatabase::new(2160);
        let candidates = db.get_candidates(None, None, None, None, None);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_add_template() {
        let mut db = TemplateDatabase::new(2160);

        let template = IconTemplate::builder()
            .image(vec![0u8; 64 * 64 * 3])
            .code("rifle_001")
            .mod_name("vanilla")
            .faction(ItemFaction::Neutral)
            .category(ItemCategory::Item)
            .crated(false)
            .phash(12345)
            .build();

        db.add_template(template);

        assert_eq!(db.templates.len(), 1);
        assert!(db.faction_lookup.contains_key(&ItemFaction::Neutral));
        assert!(db.category_lookup.contains_key(&ItemCategory::Item));
        assert!(db.mod_lookup.contains_key("vanilla"));
    }

    #[test]
    fn test_filter_by_faction() {
        let mut db = TemplateDatabase::new(2160);

        // Add neutral item
        db.add_template(
            IconTemplate::builder()
                .image(vec![])
                .code("neutral_item")
                .mod_name("vanilla")
                .faction(ItemFaction::Neutral)
                .category(ItemCategory::Item)
                .crated(false)
                .phash(1)
                .build(),
        );

        // Add colonial item
        db.add_template(
            IconTemplate::builder()
                .image(vec![])
                .code("colonial_item")
                .mod_name("vanilla")
                .faction(ItemFaction::Colonials)
                .category(ItemCategory::Item)
                .crated(false)
                .phash(2)
                .build(),
        );

        // Add warden item
        db.add_template(
            IconTemplate::builder()
                .image(vec![])
                .code("warden_item")
                .mod_name("vanilla")
                .faction(ItemFaction::Wardens)
                .category(ItemCategory::Item)
                .crated(false)
                .phash(3)
                .build(),
        );

        // Colonial filter should return neutral + colonial
        let candidates = db.get_candidates(Some(ItemFaction::Colonials), None, None, None, None);
        assert_eq!(candidates.len(), 2);

        // Warden filter should return neutral + warden
        let candidates = db.get_candidates(Some(ItemFaction::Wardens), None, None, None, None);
        assert_eq!(candidates.len(), 2);

        // No filter should return all
        let candidates = db.get_candidates(None, None, None, None, None);
        assert_eq!(candidates.len(), 3);
    }
}

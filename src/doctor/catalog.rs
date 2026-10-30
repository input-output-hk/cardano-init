//! Loads the embedded `registry/deps.toml` into a `DepCatalog` (tech-spec §9.2).
//!
//! Recipes are data; the installers they reference are code. Unknown installer
//! keys are rejected at load.

use std::collections::HashMap;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::installers::Installer;

#[derive(RustEmbed)]
#[folder = "registry/"]
#[include = "deps.toml"]
struct DepsAsset;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("registry/deps.toml is missing or not valid UTF-8")]
    Missing,

    #[error("failed to parse registry/deps.toml: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("dep '{dep}' has a malformed install entry (expected a single {{ installer = arg }})")]
    MalformedInstall { dep: String },

    #[error("unknown installer '{installer}' in dep '{dep}'")]
    UnknownInstaller { dep: String, installer: String },
}

/// A single install method: which installer, and the arg to pass it.
#[derive(Debug, Clone)]
pub struct InstallMethod {
    pub installer: Installer,
    pub arg: String,
}

pub use crate::registry::types::Link as SupportLink;

/// A per-dependency recipe.
#[derive(Debug, Clone)]
pub struct DepRecipe {
    /// The dep is present if any of these is on `PATH`.
    pub binaries: Vec<String>,
    /// Fallback advice when no install plan can be resolved.
    pub docs: String,
    /// Install methods in preference order.
    pub install: Vec<InstallMethod>,
    pub support: Vec<SupportLink>,
}

/// All dep recipes, keyed by dep id.
#[derive(Debug, Clone)]
pub struct DepCatalog {
    recipes: HashMap<String, DepRecipe>,
}

#[derive(Deserialize)]
struct DepRecipeToml {
    binaries: Vec<String>,
    docs: String,
    /// Each entry is a single-key table: `{ brew = "node" }`.
    install: Vec<HashMap<String, String>>,
    #[serde(default)]
    support: Vec<SupportLink>,
}

impl DepRecipeToml {
    fn into_recipe(self, dep: &str) -> Result<DepRecipe, CatalogError> {
        let install = self
            .install
            .into_iter()
            .map(|entry| parse_install_method(dep, entry))
            .collect::<Result<_, _>>()?;
        Ok(DepRecipe {
            binaries: self.binaries,
            docs: self.docs,
            install,
            support: self.support,
        })
    }
}

fn parse_install_method(
    dep: &str,
    entry: HashMap<String, String>,
) -> Result<InstallMethod, CatalogError> {
    let mut pairs = entry.into_iter();
    let (Some((key, arg)), None) = (pairs.next(), pairs.next()) else {
        return Err(CatalogError::MalformedInstall {
            dep: dep.to_string(),
        });
    };
    let installer = Installer::from_key(&key).ok_or_else(|| CatalogError::UnknownInstaller {
        dep: dep.to_string(),
        installer: key,
    })?;
    Ok(InstallMethod { installer, arg })
}

impl DepCatalog {
    /// Load and validate the embedded `registry/deps.toml`.
    pub fn load() -> Result<Self, CatalogError> {
        let data = DepsAsset::get("deps.toml").ok_or(CatalogError::Missing)?;
        let text = std::str::from_utf8(&data.data).map_err(|_| CatalogError::Missing)?;
        Self::from_str(text)
    }

    /// Parse and validate a catalog from TOML text.
    pub fn from_str(text: &str) -> Result<Self, CatalogError> {
        let raw: HashMap<String, DepRecipeToml> = toml::from_str(text)?;
        let recipes = raw
            .into_iter()
            .map(|(dep, recipe)| {
                let recipe = recipe.into_recipe(&dep)?;
                Ok((dep, recipe))
            })
            .collect::<Result<_, CatalogError>>()?;
        Ok(DepCatalog { recipes })
    }

    /// Look up a recipe by dep id.
    pub fn get(&self, dep_id: &str) -> Option<&DepRecipe> {
        self.recipes.get(dep_id)
    }

    /// All dep ids, in unspecified order.
    pub fn dep_ids(&self) -> impl Iterator<Item = &str> {
        self.recipes.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_loads() {
        let cat = DepCatalog::load().expect("registry/deps.toml should load");
        // Base dep + a couple of tool deps must exist.
        assert!(cat.get("just").is_some());
        assert!(cat.get("node").is_some());
        assert!(cat.get("aiken").is_some());
    }

    #[test]
    fn aiken_recipe_shape() {
        let cat = DepCatalog::load().unwrap();
        let aiken = cat.get("aiken").unwrap();
        assert_eq!(aiken.binaries, vec!["aiken".to_string()]);
        // First method is aikup, second is nix (order = preference).
        assert_eq!(aiken.install[0].installer, Installer::Aikup);
        // Empty arg ⇒ `aikup install` (latest); "latest" is not a valid tag.
        assert_eq!(aiken.install[0].arg, "");
        assert_eq!(aiken.install[1].installer, Installer::Nix);
    }

    #[test]
    fn support_defaults_to_empty() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "foo" }]
"#;
        let cat = DepCatalog::from_str(toml).unwrap();
        assert!(cat.get("foo").unwrap().support.is_empty());
    }

    #[test]
    fn aiken_has_support_links() {
        let cat = DepCatalog::load().unwrap();
        assert!(!cat.get("aiken").unwrap().support.is_empty());
    }

    #[test]
    fn support_urls_are_https() {
        let cat = DepCatalog::load().unwrap();
        for id in cat.dep_ids() {
            for link in &cat.get(id).unwrap().support {
                assert!(
                    !link.name.is_empty(),
                    "dep '{id}' has an unnamed support link"
                );
                assert!(
                    link.url.starts_with("https://"),
                    "dep '{id}' support link '{}' is not https",
                    link.name
                );
            }
        }
    }

    #[test]
    fn unknown_installer_is_load_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ snap = "foo" }]
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::UnknownInstaller { .. }));
    }

    #[test]
    fn multi_key_install_entry_is_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "foo", apt = "foo" }]
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::MalformedInstall { .. }));
    }
}

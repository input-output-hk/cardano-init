//! Loads the embedded `registry/deps.toml` into a `DepCatalog` (tech-spec §9.2).
//!
//! Recipes are data: each method's command is a MiniJinja template rendered
//! with the pinned `version`. The installers they reference are code (detection
//! and bootstrap); unknown installer keys and broken templates are rejected at
//! load.

use std::collections::HashMap;
use std::sync::OnceLock;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::installers::Installer;
use crate::registry::version::VersionReq;

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

    #[error(
        "dep '{dep}' has a malformed install entry (expected a single {{ installer = \"command\" }})"
    )]
    MalformedInstall { dep: String },

    #[error("unknown installer '{installer}' in dep '{dep}'")]
    UnknownInstaller { dep: String, installer: String },

    #[error("dep '{dep}' has an invalid command template for '{installer}': {source}")]
    InvalidTemplate {
        dep: String,
        installer: String,
        source: minijinja::Error,
    },
}

/// A single install (or upgrade) method: which installer, and the command
/// template that runs it.
#[derive(Debug, Clone)]
pub struct InstallMethod {
    pub installer: Installer,
    pub template: String,
    /// The template uses `version`, so the command installs the pin rather
    /// than whatever the installer's source has as latest.
    pub uses_version: bool,
}

impl InstallMethod {
    /// The command for this method, with `version` filled in (empty when
    /// unpinned).
    pub fn render(&self, version: Option<&VersionReq>) -> String {
        render(&self.template, version).expect("templates are validated at load")
    }
}

fn template_env() -> &'static minijinja::Environment<'static> {
    static ENV: OnceLock<minijinja::Environment<'static>> = OnceLock::new();
    ENV.get_or_init(build_template_env)
}

fn build_template_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
    // `prefix(p)` prepends `p` to a non-empty value and keeps an empty one
    // empty, so `pkg{{ version | prefix('@') }}` never renders a dangling `@`.
    env.add_filter("prefix", |value: String, p: String| {
        if value.is_empty() {
            value
        } else {
            format!("{p}{value}")
        }
    });
    env
}

fn render(template: &str, version: Option<&VersionReq>) -> Result<String, minijinja::Error> {
    let version = version.map(ToString::to_string).unwrap_or_default();
    let out = template_env().render_str(template, minijinja::context! { version })?;
    Ok(out.trim().to_string())
}

pub use crate::registry::types::Link as SupportLink;

/// How to read an installed dep's version (tech-spec §9.7). Only run for deps
/// a selected tool pins.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionProbe {
    /// Run the dep's binary with these args and take the first dotted version
    /// in its output. The default is `--version`.
    Args(Vec<String>),
    /// Read `version` from this npm package's `package.json` behind the
    /// binary, without running it. For CLIs whose `--version` has side effects
    /// (`yaci-devkit --version` boots the CLI and kills a running devnet).
    Npm(String),
}

impl Default for VersionProbe {
    fn default() -> Self {
        VersionProbe::Args(vec!["--version".to_string()])
    }
}

/// A per-dependency recipe.
#[derive(Debug, Clone)]
pub struct DepRecipe {
    /// The dep is present if any of these is on `PATH`.
    pub binaries: Vec<String>,
    /// Fallback advice when no install plan can be resolved.
    pub docs: String,
    /// Install methods in preference order.
    pub install: Vec<InstallMethod>,
    /// Per-installer upgrade overrides; see [`DepRecipe::upgrade_method`].
    pub upgrade: Vec<InstallMethod>,
    pub support: Vec<SupportLink>,
    pub version: VersionProbe,
    /// A command to run after upgrading, chained onto every upgrade command
    /// (e.g. refreshing components the dep downloaded itself).
    pub after_upgrade: Option<String>,
}

impl DepRecipe {
    /// The method that moves an installed dep to the pinned (or latest)
    /// version through `install`'s installer: its `upgrade` override, else the
    /// install command itself.
    pub fn upgrade_method<'a>(&'a self, install: &'a InstallMethod) -> &'a InstallMethod {
        self.upgrade
            .iter()
            .find(|m| m.installer == install.installer)
            .unwrap_or(install)
    }
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
    /// Each entry is a single-key table: `{ brew = "brew install node" }`.
    install: Vec<HashMap<String, String>>,
    #[serde(default)]
    upgrade: Vec<HashMap<String, String>>,
    #[serde(default)]
    support: Vec<SupportLink>,
    #[serde(default)]
    version: VersionProbe,
    after_upgrade: Option<String>,
}

impl DepRecipeToml {
    fn into_recipe(self, dep: &str) -> Result<DepRecipe, CatalogError> {
        let parse = |entries: Vec<HashMap<String, String>>| {
            entries
                .into_iter()
                .map(|entry| parse_install_method(dep, entry))
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(DepRecipe {
            binaries: self.binaries,
            docs: self.docs,
            install: parse(self.install)?,
            upgrade: parse(self.upgrade)?,
            support: self.support,
            version: self.version,
            after_upgrade: self.after_upgrade,
        })
    }
}

fn parse_install_method(
    dep: &str,
    entry: HashMap<String, String>,
) -> Result<InstallMethod, CatalogError> {
    let mut pairs = entry.into_iter();
    let (Some((key, template)), None) = (pairs.next(), pairs.next()) else {
        return Err(CatalogError::MalformedInstall {
            dep: dep.to_string(),
        });
    };
    let installer = Installer::from_key(&key).ok_or_else(|| CatalogError::UnknownInstaller {
        dep: dep.to_string(),
        installer: key.clone(),
    })?;
    let pinned = VersionReq::parse("0");
    let (unpinned, pinned) = render(&template, None)
        .and_then(|bare| Ok((bare, render(&template, pinned.as_ref())?)))
        .map_err(|source| CatalogError::InvalidTemplate {
            dep: dep.to_string(),
            installer: key,
            source,
        })?;
    Ok(InstallMethod {
        installer,
        template,
        uses_version: unpinned != pinned,
    })
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
        assert_eq!(aiken.install[1].installer, Installer::Nix);
    }

    #[test]
    fn templates_render_with_and_without_version() {
        let cat = DepCatalog::load().unwrap();
        let aikup = &cat.get("aiken").unwrap().install[0];
        let pin = VersionReq::parse("1.1.24");
        assert_eq!(aikup.render(pin.as_ref()), "aikup install v1.1.24");
        // Unpinned ⇒ bare `aikup install` (latest), no trailing space.
        assert_eq!(aikup.render(None), "aikup install");

        let yaci = cat.get("yaci-devkit").unwrap();
        let upgrade = yaci.upgrade_method(&yaci.install[0]);
        let pin = VersionReq::parse("0.12");
        assert_eq!(
            upgrade.render(pin.as_ref()),
            "npm install -g @bloxbean/yaci-devkit@0.12"
        );
        assert_eq!(
            upgrade.render(None),
            "npm install -g @bloxbean/yaci-devkit@latest"
        );
        assert_eq!(
            yaci.install[0].render(None),
            "npm install -g @bloxbean/yaci-devkit"
        );
    }

    #[test]
    fn upgrade_falls_back_to_install() {
        let cat = DepCatalog::load().unwrap();
        let aiken = cat.get("aiken").unwrap();
        let aikup = &aiken.install[0];
        assert_eq!(aiken.upgrade_method(aikup).template, aikup.template);
        let nix = &aiken.install[1];
        assert_eq!(
            aiken.upgrade_method(nix).render(None),
            "nix profile upgrade aiken"
        );
    }

    #[test]
    fn uses_version_tracks_the_template() {
        let cat = DepCatalog::load().unwrap();
        let aiken = cat.get("aiken").unwrap();
        assert!(aiken.install[0].uses_version);
        assert!(!aiken.install[1].uses_version);
        assert!(!aiken.upgrade[0].uses_version);
    }

    #[test]
    fn broken_template_is_load_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "brew install foo{{ version" }]
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::InvalidTemplate { .. }));
    }

    #[test]
    fn unknown_template_variable_is_load_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "brew install {{ package }}" }]
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::InvalidTemplate { .. }));
    }

    #[test]
    fn support_defaults_to_empty() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "brew install foo" }]
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
    fn version_probe_defaults_to_dash_dash_version() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "brew install foo" }]
"#;
        let cat = DepCatalog::from_str(toml).unwrap();
        assert_eq!(cat.get("foo").unwrap().version, VersionProbe::default());
    }

    #[test]
    fn version_probe_overrides() {
        let cat = DepCatalog::load().unwrap();
        assert_eq!(
            cat.get("yaci-devkit").unwrap().version,
            VersionProbe::Npm("@bloxbean/yaci-devkit".to_string())
        );
        assert_eq!(
            cat.get("cardano-up").unwrap().version,
            VersionProbe::Args(vec!["version".to_string()])
        );
    }

    #[test]
    fn unknown_installer_is_load_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ snap = "snap install foo" }]
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

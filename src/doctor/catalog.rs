//! Loads the embedded `registry/deps.toml` and `registry/installers.toml` into
//! a `DepCatalog` (tech-spec §9.2).
//!
//! Both are data: each recipe method names a declared installer and a MiniJinja
//! command template rendered with the pinned `version`. Undeclared installers,
//! bootstrap ids without a recipe, and broken templates are rejected at load.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::installers::{Installer, InstallerDef};
use crate::registry::version::VersionReq;

#[derive(RustEmbed)]
#[folder = "registry/"]
#[include = "deps.toml"]
#[include = "installers.toml"]
struct DepsAsset;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("registry/{0} is missing or not valid UTF-8")]
    Missing(&'static str),

    #[error("failed to parse registry/deps.toml: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("failed to parse registry/installers.toml: {0}")]
    ParseInstallers(#[source] toml::de::Error),

    #[error("installer '{installer}' has an empty detect list")]
    EmptyDetect { installer: String },

    #[error(
        "installer '{installer}' bootstraps via dep '{dep}', which has no registry/deps.toml entry"
    )]
    UnknownBootstrapDep { installer: String, dep: String },

    #[error("installer '{installer}' is declared but no registry/deps.toml method uses it")]
    UnusedInstaller { installer: String },

    #[error(
        "dep '{dep}' has a malformed install entry (expected a single {{ installer = \"command\" }})"
    )]
    MalformedInstall { dep: String },

    #[error(
        "dep '{dep}' uses installer '{installer}', which registry/installers.toml doesn't declare"
    )]
    UnknownInstaller { dep: String, installer: String },

    #[error(
        "dep '{dep}' has an upgrade override for '{installer}', which is not in its install list"
    )]
    UpgradeWithoutInstall { dep: String, installer: String },

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

/// Build the strict MiniJinja environment with a `prefix(p)` filter that
/// prepends `p` only to non-empty values so optional version flags never leave
/// a dangling separator.
fn build_template_env() -> minijinja::Environment<'static> {
    let mut env = minijinja::Environment::new();
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
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

fn embedded(file: &'static str) -> Result<String, CatalogError> {
    let data = DepsAsset::get(file).ok_or(CatalogError::Missing(file))?;
    String::from_utf8(data.data.into_owned()).map_err(|_| CatalogError::Missing(file))
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

/// All dep recipes, keyed by dep id, and the installers they use.
#[derive(Debug, Clone)]
pub struct DepCatalog {
    recipes: HashMap<String, DepRecipe>,
    installers: BTreeMap<Installer, InstallerDef>,
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
    fn into_recipe(
        self,
        dep: &str,
        installers: &BTreeMap<Installer, InstallerDef>,
    ) -> Result<DepRecipe, CatalogError> {
        let parse = |entries: Vec<HashMap<String, String>>| {
            entries
                .into_iter()
                .map(|entry| parse_install_method(dep, entry, installers))
                .collect::<Result<Vec<_>, _>>()
        };
        let install = parse(self.install)?;
        let upgrade = parse(self.upgrade)?;
        if let Some(m) = upgrade
            .iter()
            .find(|u| !install.iter().any(|i| i.installer == u.installer))
        {
            return Err(CatalogError::UpgradeWithoutInstall {
                dep: dep.to_string(),
                installer: m.installer.to_string(),
            });
        }
        Ok(DepRecipe {
            binaries: self.binaries,
            docs: self.docs,
            install,
            upgrade,
            support: self.support,
            version: self.version,
            after_upgrade: self.after_upgrade,
        })
    }
}

fn parse_install_method(
    dep: &str,
    entry: HashMap<String, String>,
    installers: &BTreeMap<Installer, InstallerDef>,
) -> Result<InstallMethod, CatalogError> {
    let mut pairs = entry.into_iter();
    let (Some((key, template)), None) = (pairs.next(), pairs.next()) else {
        return Err(CatalogError::MalformedInstall {
            dep: dep.to_string(),
        });
    };
    let installer = Installer::new(key.clone());
    if !installers.contains_key(&installer) {
        return Err(CatalogError::UnknownInstaller {
            dep: dep.to_string(),
            installer: key,
        });
    }
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
    /// Load and validate the embedded `registry/deps.toml` and
    /// `registry/installers.toml`.
    pub fn load() -> Result<Self, CatalogError> {
        Self::from_parts(&embedded("deps.toml")?, &embedded("installers.toml")?)
    }

    /// Parse and validate recipes from TOML text, with a detect-only `brew`
    /// installer (enough for the small catalogs tests build).
    #[cfg(test)]
    pub fn from_str(deps: &str) -> Result<Self, CatalogError> {
        Self::from_parts(deps, "[brew]\ndetect = [\"brew\"]\n")
    }

    /// Parse and validate a catalog from `deps.toml` and `installers.toml` text.
    pub fn from_parts(deps: &str, installers: &str) -> Result<Self, CatalogError> {
        let installers: BTreeMap<String, InstallerDef> =
            toml::from_str(installers).map_err(CatalogError::ParseInstallers)?;
        let installers: BTreeMap<Installer, InstallerDef> = installers
            .into_iter()
            .map(|(key, def)| (Installer::new(key), def))
            .collect();
        let raw: HashMap<String, DepRecipeToml> = toml::from_str(deps)?;
        let recipes: HashMap<String, DepRecipe> = raw
            .into_iter()
            .map(|(dep, recipe)| {
                let recipe = recipe.into_recipe(&dep, &installers)?;
                Ok((dep, recipe))
            })
            .collect::<Result<_, CatalogError>>()?;
        for (installer, def) in &installers {
            if def.detect.is_empty() {
                return Err(CatalogError::EmptyDetect {
                    installer: installer.to_string(),
                });
            }
            if let Some(dep) = def.bootstrap.iter().find(|d| !recipes.contains_key(*d)) {
                return Err(CatalogError::UnknownBootstrapDep {
                    installer: installer.to_string(),
                    dep: dep.clone(),
                });
            }
            let used = recipes
                .values()
                .any(|r| r.install.iter().any(|m| &m.installer == installer));
            if !used {
                return Err(CatalogError::UnusedInstaller {
                    installer: installer.to_string(),
                });
            }
        }
        Ok(DepCatalog {
            recipes,
            installers,
        })
    }

    /// Keep only `installer`'s method in `dep`'s recipe, for tests of the
    /// fallback when nothing installs a pin.
    #[cfg(test)]
    pub fn retain_install_for_test(&mut self, dep: &str, installer: &str) {
        if let Some(recipe) = self.recipes.get_mut(dep) {
            recipe.install.retain(|m| m.installer == installer);
        }
    }

    /// The declared installers, in id order.
    pub fn installers(&self) -> impl Iterator<Item = (&Installer, &InstallerDef)> {
        self.installers.iter()
    }

    /// Dep ids that provide `installer`; empty when it is detect-only.
    pub fn bootstrap(&self, installer: &Installer) -> &[String] {
        self.installers
            .get(installer)
            .map_or(&[][..], |def| &def.bootstrap)
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
        assert_eq!(aiken.install[0].installer, "aikup");
        assert_eq!(aiken.install[1].installer, "nix");
    }

    #[test]
    fn templates_render_with_and_without_version() {
        let cat = DepCatalog::load().unwrap();
        let aikup = &cat.get("aiken").unwrap().install[0];
        let pin = VersionReq::parse("1.1.24");
        assert_eq!(aikup.render(pin.as_ref()), "aikup install v1.1.24");
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

    const FOO: &str = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ fooup = "fooup install" }]
"#;

    #[test]
    fn embedded_installers_load_with_bootstraps() {
        let cat = DepCatalog::load().unwrap();
        assert_eq!(cat.bootstrap(&Installer::new("npm")), ["node"]);
        assert!(cat.bootstrap(&Installer::new("brew")).is_empty());
        let (_, ps) = cat.installers().find(|(i, _)| **i == "powershell").unwrap();
        assert_eq!(ps.detect, ["powershell", "pwsh"]);
    }

    #[test]
    fn a_declared_installer_needs_no_code() {
        let cat = DepCatalog::from_parts(
            FOO,
            "[fooup]\ndetect = [\"fooup\"]\nbootstrap = [\"foo\"]\n",
        )
        .unwrap();
        assert_eq!(cat.get("foo").unwrap().install[0].installer, "fooup");
        assert_eq!(cat.bootstrap(&Installer::new("fooup")), ["foo"]);
    }

    #[test]
    fn bootstrap_without_recipe_is_load_error() {
        let err = DepCatalog::from_parts(
            FOO,
            "[fooup]\ndetect = [\"fooup\"]\nbootstrap = [\"bar\"]\n",
        )
        .unwrap_err();
        assert!(matches!(err, CatalogError::UnknownBootstrapDep { .. }));
    }

    #[test]
    fn empty_detect_is_load_error() {
        let err = DepCatalog::from_parts(FOO, "[fooup]\ndetect = []\n").unwrap_err();
        assert!(matches!(err, CatalogError::EmptyDetect { .. }));
    }

    #[test]
    fn unused_installer_is_load_error() {
        let err = DepCatalog::from_parts(
            FOO,
            "[fooup]\ndetect = [\"fooup\"]\n[snap]\ndetect = [\"snap\"]\n",
        )
        .unwrap_err();
        assert!(matches!(err, CatalogError::UnusedInstaller { installer } if installer == "snap"));
    }

    #[test]
    fn unknown_installer_field_is_load_error() {
        let err = DepCatalog::from_parts(FOO, "[fooup]\ndetect = [\"fooup\"]\nbootstap = []\n")
            .unwrap_err();
        assert!(matches!(err, CatalogError::ParseInstallers(_)));
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

    #[test]
    fn upgrade_without_matching_install_is_load_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "brew install foo" }]
upgrade = [{ apt = "sudo apt install --only-upgrade -y foo" }]
"#;
        let installers = "[brew]\ndetect = [\"brew\"]\n[apt]\ndetect = [\"apt\"]\n";
        let err = DepCatalog::from_parts(toml, installers).unwrap_err();
        assert!(matches!(
            err,
            CatalogError::UpgradeWithoutInstall { dep, installer }
            if dep == "foo" && installer == "apt"
        ));
    }
}

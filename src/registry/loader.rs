use std::collections::{BTreeMap, HashMap};

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::types::{
    CompatConfig, DetectSignature, Era, InfraConfig, Link, Role, RoleConfig, Seam, SystemDep,
    ToolDef, UnknownEraError, UnknownRoleError, UnknownSeamError,
};
use super::version::VersionReq;

#[derive(RustEmbed)]
#[folder = "registry/"]
struct RegistryAssets;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("failed to parse tool file '{file}': {source}")]
    Parse {
        file: String,
        source: toml::de::Error,
    },

    #[error("unknown role '{role}' in tool file '{file}': {source}")]
    UnknownRole {
        file: String,
        role: String,
        source: UnknownRoleError,
    },

    #[error("unknown seam '{seam}' in tool file '{file}': {source}")]
    UnknownSeam {
        file: String,
        seam: String,
        source: UnknownSeamError,
    },

    #[error("unknown era '{era}' in tool file '{file}': {source}")]
    UnknownEra {
        file: String,
        era: String,
        source: UnknownEraError,
    },

    #[error(
        "tool '{file}' has a malformed system_deps entry \
         (expected \"dep\" or a single {{ dep = \"version\" }})"
    )]
    MalformedSystemDep { file: String },

    #[error(
        "tool '{file}' has an invalid version '{version}' for '{dep}' \
         (expected dotted numbers, e.g. \"1.1.24\" or \"1.1\", or a floor, e.g. \">=20\")"
    )]
    InvalidDepVersion {
        file: String,
        dep: String,
        version: String,
    },

    #[error("duplicate tool id '{id}'")]
    DuplicateId { id: String },

    #[error(
        "tool '{file}' fills the infrastructure role but is missing the required [infra] table"
    )]
    InfraConfigMissing { file: String },

    #[error(
        "tool '{file}' declares a [fullstack] template but must also declare both \
         [roles.on-chain] and [roles.off-chain]"
    )]
    FullstackRolesMissing { file: String },

    #[error("no tool definitions found in registry")]
    Empty,
}

/// On-disk shape of `registry/tools/<tool>.toml`.
#[derive(Deserialize)]
struct ToolFileToml {
    tool: ToolMetaToml,
    #[serde(default)]
    roles: HashMap<String, RoleConfig>,
    infra: Option<InfraConfig>,
    fullstack: Option<RoleConfig>,
    #[serde(default)]
    compat: CompatToml,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolMetaToml {
    id: String,
    name: String,
    description: String,
    website: String,
    #[serde(default)]
    community: Vec<Link>,
    languages: Vec<String>,
    era: String,
    #[serde(default)]
    system_deps: Vec<DepToml>,
    #[serde(default)]
    nix_packages: Vec<String>,
    #[serde(default)]
    nix_self_contained: bool,
    #[serde(default)]
    detect: Vec<DetectToml>,
    #[serde(default)]
    experimental: bool,
}

#[derive(Deserialize, Default)]
struct CompatToml {
    #[serde(default)]
    consumes: Vec<String>,
    #[serde(default)]
    serves: Vec<String>,
    #[serde(default)]
    self_contained_devnet: bool,
}

/// A `system_deps` entry: a bare dep id, or a single `{ dep = "version" }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum DepToml {
    Bare(String),
    Pinned(BTreeMap<String, String>),
}

/// A `detect` entry is either a bare path (existence check) or a table with an
/// optional `contains` substring (content check).
#[derive(Deserialize)]
#[serde(untagged)]
enum DetectToml {
    Simple(String),
    Detailed {
        file: String,
        #[serde(default)]
        contains: Option<String>,
    },
}

impl From<DetectToml> for DetectSignature {
    fn from(raw: DetectToml) -> Self {
        match raw {
            DetectToml::Simple(file) => DetectSignature {
                file,
                contains: None,
            },
            DetectToml::Detailed { file, contains } => DetectSignature { file, contains },
        }
    }
}

fn parse_tool_file(file_name: &str) -> Result<ToolDef, RegistryError> {
    let asset = RegistryAssets::get(file_name).expect("asset listed by iter() must exist");
    let text = std::str::from_utf8(&asset.data).expect("tool TOML files must be valid UTF-8");
    let raw = toml::from_str(text).map_err(|source| RegistryError::Parse {
        file: file_name.to_string(),
        source,
    })?;
    to_tool_def(file_name, raw)
}

/// Convert and validate a parsed tool file.
fn to_tool_def(file_name: &str, raw: ToolFileToml) -> Result<ToolDef, RegistryError> {
    let roles = parse_roles(file_name, raw.roles)?;

    if roles.contains_key(&Role::Infrastructure) && raw.infra.is_none() {
        return Err(RegistryError::InfraConfigMissing {
            file: file_name.to_string(),
        });
    }

    // `[fullstack]` fuses on-chain + off-chain, so the tool must fill both.
    if raw.fullstack.is_some()
        && !(roles.contains_key(&Role::OnChain) && roles.contains_key(&Role::OffChain))
    {
        return Err(RegistryError::FullstackRolesMissing {
            file: file_name.to_string(),
        });
    }

    let compat = CompatConfig {
        consumes: parse_seams(file_name, &raw.compat.consumes)?,
        serves: parse_seams(file_name, &raw.compat.serves)?,
        self_contained_devnet: raw.compat.self_contained_devnet,
    };

    let ToolMetaToml {
        id,
        name,
        description,
        website,
        community,
        languages,
        era,
        system_deps,
        nix_packages,
        nix_self_contained,
        detect,
        experimental,
    } = raw.tool;
    let era = Era::from_kebab(&era).map_err(|source| RegistryError::UnknownEra {
        file: file_name.to_string(),
        era,
        source,
    })?;
    let system_deps = system_deps
        .into_iter()
        .map(|dep| parse_system_dep(file_name, dep))
        .collect::<Result<_, _>>()?;

    Ok(ToolDef {
        id,
        name,
        description,
        website,
        community,
        languages,
        era,
        system_deps,
        nix_packages,
        nix_self_contained,
        detect: detect.into_iter().map(DetectSignature::from).collect(),
        roles,
        infra: raw.infra,
        experimental,
        fullstack: raw.fullstack,
        compat,
    })
}

fn parse_system_dep(file_name: &str, raw: DepToml) -> Result<SystemDep, RegistryError> {
    let (id, version) = match raw {
        DepToml::Bare(id) => return Ok(SystemDep { id, version: None }),
        DepToml::Pinned(table) if table.len() == 1 => table.into_iter().next().unwrap(),
        DepToml::Pinned(_) => {
            return Err(RegistryError::MalformedSystemDep {
                file: file_name.to_string(),
            });
        }
    };
    match VersionReq::parse(&version) {
        Some(req) => Ok(SystemDep {
            id,
            version: Some(req),
        }),
        None => Err(RegistryError::InvalidDepVersion {
            file: file_name.to_string(),
            dep: id,
            version,
        }),
    }
}

fn parse_roles(
    file_name: &str,
    raw: HashMap<String, RoleConfig>,
) -> Result<HashMap<Role, RoleConfig>, RegistryError> {
    raw.into_iter()
        .map(|(role, cfg)| match Role::from_kebab(&role) {
            Ok(parsed) => Ok((parsed, cfg)),
            Err(source) => Err(RegistryError::UnknownRole {
                file: file_name.to_string(),
                role,
                source,
            }),
        })
        .collect()
}

fn parse_seams(file_name: &str, raw: &[String]) -> Result<Vec<Seam>, RegistryError> {
    raw.iter()
        .map(|s| {
            Seam::from_kebab(s).map_err(|source| RegistryError::UnknownSeam {
                file: file_name.to_string(),
                seam: s.clone(),
                source,
            })
        })
        .collect()
}

/// Immutable registry of all tool definitions, loaded from embedded TOML files.
pub struct Registry {
    tools: Vec<ToolDef>,
    by_id: HashMap<String, usize>,
    by_role: HashMap<Role, Vec<usize>>,
}

impl Registry {
    /// Load all tool definitions from the embedded `registry/tools/` directory.
    ///
    /// Fails on malformed TOML, unknown roles/seams, inconsistent tables,
    /// duplicate ids, or an empty registry.
    pub fn load() -> Result<Self, RegistryError> {
        let mut registry = Registry {
            tools: Vec::new(),
            by_id: HashMap::new(),
            by_role: HashMap::new(),
        };

        let tool_files = RegistryAssets::iter()
            .filter(|name| name.starts_with("tools/") && name.ends_with(".toml"));
        for file_name in tool_files {
            registry.insert(parse_tool_file(&file_name)?)?;
        }

        if registry.tools.is_empty() {
            return Err(RegistryError::Empty);
        }
        Ok(registry)
    }

    fn insert(&mut self, tool: ToolDef) -> Result<(), RegistryError> {
        if self.by_id.contains_key(&tool.id) {
            return Err(RegistryError::DuplicateId { id: tool.id });
        }

        let idx = self.tools.len();
        self.by_id.insert(tool.id.clone(), idx);
        for &role in tool.roles.keys() {
            self.by_role.entry(role).or_default().push(idx);
        }
        self.tools.push(tool);
        Ok(())
    }

    /// Look up a tool by its id.
    pub fn get(&self, id: &str) -> Option<&ToolDef> {
        self.by_id.get(id).map(|&idx| &self.tools[idx])
    }

    /// All tools that can fill the given role.
    pub fn tools_for_role(&self, role: Role) -> Vec<&ToolDef> {
        self.by_role
            .get(&role)
            .map(|indices| indices.iter().map(|&idx| &self.tools[idx]).collect())
            .unwrap_or_default()
    }

    /// All loaded tool definitions.
    pub fn all_tools(&self) -> &[ToolDef] {
        &self.tools
    }

    /// Re-park a tool on another era, for tests of mixed-era selections (the
    /// shipped registry has a single era).
    #[cfg(test)]
    pub(crate) fn set_era_for_test(&mut self, id: &str, era: Era) {
        let idx = self.by_id[id];
        self.tools[idx].era = era;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load successfully")
    }

    fn sorted_ids_for_role(reg: &Registry, role: Role) -> Vec<&str> {
        let mut ids: Vec<&str> = reg
            .tools_for_role(role)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Parse a demo tool file: a fixed `[tool]` header followed by `tables`.
    fn demo_tool(tables: &str) -> Result<ToolDef, RegistryError> {
        let toml = format!(
            r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]
            era = "conway"
            {tables}
            "#
        );
        let raw: ToolFileToml = toml::from_str(&toml).unwrap();
        to_tool_def("demo.toml", raw)
    }

    #[test]
    fn load_succeeds() {
        let _ = registry();
    }

    #[test]
    fn load_tool_count() {
        // aiken, pebble, plinth, plutarch, scalus, meshjs, evolution, lucid, blaze,
        // apollo, pycardano, tx3, yaci, blaster + infra: kupo, ogmios, dolos, tx-submit-api,
        // cardano-node, cardano-node-api, dingo
        assert_eq!(registry().all_tools().len(), 21);
    }

    #[test]
    fn get_by_id() {
        let reg = registry();
        let aiken = reg.get("aiken").expect("aiken should exist");
        assert_eq!(aiken.name, "Aiken");
        assert_eq!(aiken.id, "aiken");
    }

    #[test]
    fn get_unknown_returns_none() {
        assert!(registry().get("nonexistent").is_none());
    }

    #[test]
    fn tools_for_role_on_chain() {
        assert_eq!(
            sorted_ids_for_role(&registry(), Role::OnChain),
            vec!["aiken", "pebble", "plinth", "plutarch", "scalus"]
        );
    }

    #[test]
    fn tools_for_role_off_chain() {
        assert_eq!(
            sorted_ids_for_role(&registry(), Role::OffChain),
            vec![
                "apollo",
                "blaze",
                "evolution",
                "lucid",
                "meshjs",
                "pycardano",
                "scalus",
                "tx3"
            ]
        );
    }

    #[test]
    fn tools_for_role_devnet() {
        // Yaci DevKit is a local devnet, so it fills the devnet role (not infra).
        assert_eq!(sorted_ids_for_role(&registry(), Role::Devnet), vec!["yaci"]);
    }

    #[test]
    fn infrastructure_tools_present() {
        assert_eq!(
            sorted_ids_for_role(&registry(), Role::Infrastructure),
            vec![
                "cardano-node",
                "cardano-node-api",
                "dingo",
                "dolos",
                "kupo",
                "ogmios",
                "tx-submit-api",
            ]
        );
    }

    #[test]
    fn infra_tools_declare_infra_config() {
        let reg = registry();
        for tool in reg.tools_for_role(Role::Infrastructure) {
            let infra = tool
                .infra
                .as_ref()
                .unwrap_or_else(|| panic!("infra tool '{}' must declare [infra]", tool.id));
            assert!(
                !infra.cardano_up_package.is_empty(),
                "infra tool '{}' needs a cardano_up_package",
                tool.id
            );
            // All infra tools share the single aggregated driver template.
            assert_eq!(
                tool.roles.get(&Role::Infrastructure).unwrap().template,
                "_infra/cardano-up"
            );
        }
    }

    #[test]
    fn experimental_flag_defaults_false_and_marks_blaster() {
        let reg = registry();
        // Blaster is the current formal-methods experimental tool (work in progress).
        assert!(
            reg.get("blaster")
                .expect("blaster should exist")
                .experimental,
            "blaster should be marked experimental"
        );
        // Released tools default to non-experimental (no flag needed in their TOML).
        for id in ["aiken", "meshjs", "scalus", "yaci", "kupo"] {
            assert!(
                !reg.get(id).unwrap().experimental,
                "{id} should not be experimental by default"
            );
        }
    }

    #[test]
    fn experimental_defaults_false_when_flag_absent() {
        let tables = r#"
            [roles.on-chain]
            template = "demo/on-chain"
        "#;
        let def = demo_tool(tables).unwrap();
        assert!(!def.experimental);
    }

    #[test]
    fn scalus_multi_role() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        assert!(scalus.roles.contains_key(&Role::OnChain));
        assert!(scalus.roles.contains_key(&Role::OffChain));
        assert_eq!(scalus.roles.len(), 2);
    }

    #[test]
    fn scalus_declares_fullstack_template() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        let fs = scalus
            .fullstack
            .as_ref()
            .expect("scalus should declare a [fullstack] template");
        assert_eq!(fs.template, "scalus/fullstack");
        // Aiken (on-chain only) has no fullstack capability.
        assert!(reg.get("aiken").unwrap().fullstack.is_none());
    }

    #[test]
    fn fullstack_with_both_roles_loads() {
        let tables = r#"
            [roles.on-chain]
            template = "demo/on-chain"
            [roles.off-chain]
            template = "demo/off-chain"
            [fullstack]
            template = "demo/fullstack"
        "#;
        let def = demo_tool(tables).expect("valid fullstack tool");
        assert_eq!(def.fullstack.unwrap().template, "demo/fullstack");
    }

    #[test]
    fn fullstack_without_both_roles_is_rejected() {
        // [fullstack] but only on-chain declared → error (must fill both roles).
        let tables = r#"
            [roles.on-chain]
            template = "demo/on-chain"
            [fullstack]
            template = "demo/fullstack"
        "#;
        assert!(matches!(
            demo_tool(tables),
            Err(RegistryError::FullstackRolesMissing { .. })
        ));
    }

    #[test]
    fn all_fields_populated() {
        let reg = registry();
        for tool in reg.all_tools() {
            assert!(!tool.id.is_empty(), "id should not be empty");
            assert!(!tool.name.is_empty(), "name should not be empty");
            assert!(
                !tool.description.is_empty(),
                "description should not be empty"
            );
            assert!(!tool.website.is_empty(), "website should not be empty");
            for link in &tool.community {
                assert!(!link.name.is_empty(), "{}: unnamed community link", tool.id);
                assert!(
                    link.url.starts_with("https://"),
                    "{}: community link '{}' is not https",
                    tool.id,
                    link.name
                );
            }
            // Infra providers are cardano-up packages, not authored in a
            // language, so only they may leave `languages` empty.
            if !tool.roles.contains_key(&Role::Infrastructure) {
                assert!(!tool.languages.is_empty(), "languages should not be empty");
            }
            assert!(!tool.roles.is_empty(), "roles should not be empty");

            for (role, cfg) in &tool.roles {
                assert!(
                    !cfg.template.is_empty(),
                    "template for {} in role {} should not be empty",
                    tool.id,
                    role
                );
            }
        }
    }

    #[test]
    fn system_deps_parse_bare_and_pinned() {
        let tool = demo_tool(r#"system_deps = ["node", { ghc = "9.8" }]"#).unwrap();
        assert_eq!(tool.system_deps[0].id, "node");
        assert!(tool.system_deps[0].version.is_none());
        assert_eq!(tool.system_deps[1].id, "ghc");
        assert_eq!(tool.system_deps[1].version, VersionReq::parse("9.8"));
    }

    #[test]
    fn system_dep_accepts_a_floor() {
        let tool = demo_tool(r#"system_deps = [{ node = ">=20" }]"#).unwrap();
        assert_eq!(tool.system_deps[0].version, VersionReq::parse(">=20"));
    }

    #[test]
    fn multi_key_system_dep_is_error() {
        let err = demo_tool(r#"system_deps = [{ node = "18", ghc = "9.8" }]"#).unwrap_err();
        assert!(matches!(err, RegistryError::MalformedSystemDep { .. }));
    }

    #[test]
    fn malformed_dep_version_is_error() {
        let err = demo_tool(r#"system_deps = [{ node = "latest" }]"#).unwrap_err();
        assert!(matches!(err, RegistryError::InvalidDepVersion { .. }));
    }

    #[test]
    fn removed_min_versions_key_is_rejected() {
        let toml = r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]
            era = "conway"
            min_versions = { node = "18" }
        "#;
        assert!(toml::from_str::<ToolFileToml>(toml).is_err());
    }

    #[test]
    fn missing_era_is_rejected() {
        let toml = r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]
        "#;
        assert!(toml::from_str::<ToolFileToml>(toml).is_err());
    }

    #[test]
    fn unknown_era_is_error() {
        let toml = r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]
            era = "babbage"
        "#;
        let raw: ToolFileToml = toml::from_str(toml).unwrap();
        assert!(matches!(
            to_tool_def("demo.toml", raw),
            Err(RegistryError::UnknownEra { .. })
        ));
    }

    #[test]
    fn aiken_pin_matches_template_compiler_pin() {
        let template = include_str!("../../templates/aiken/on-chain/aiken.toml.jinja");
        let pin = template
            .lines()
            .find_map(|line| line.trim().strip_prefix("compiler = "))
            .expect("aiken.toml pins a compiler")
            .trim_matches('"');
        let registry = registry();
        let aiken = &registry.get("aiken").unwrap().system_deps;
        let dep = aiken.iter().find(|d| d.id == "aiken").unwrap();
        assert_eq!(dep.version, VersionReq::parse(pin));
    }
}

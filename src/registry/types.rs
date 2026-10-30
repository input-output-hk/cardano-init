use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::contract;

/// The functional roles a tool can fill within a Cardano protocol project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    OnChain,
    OffChain,
    Infrastructure,
    Devnet,
    FormalMethods,
}

impl Role {
    /// All role variants, in display order.
    pub const ALL: &[Role] = &[
        Role::OnChain,
        Role::OffChain,
        Role::Infrastructure,
        Role::Devnet,
        Role::FormalMethods,
    ];

    /// Parse from the kebab-case string used in TOML registry files.
    pub fn from_kebab(s: &str) -> Result<Self, UnknownRoleError> {
        match s {
            "on-chain" => Ok(Role::OnChain),
            "off-chain" => Ok(Role::OffChain),
            "infrastructure" => Ok(Role::Infrastructure),
            "devnet" => Ok(Role::Devnet),
            "formal-methods" => Ok(Role::FormalMethods),
            _ => Err(UnknownRoleError(s.to_string())),
        }
    }

    /// The kebab-case string used in TOML registry files.
    pub fn as_kebab(self) -> &'static str {
        match self {
            Role::OnChain => "on-chain",
            Role::OffChain => "off-chain",
            Role::Infrastructure => "infrastructure",
            Role::Devnet => "devnet",
            Role::FormalMethods => "formal-methods",
        }
    }

    /// Whether this role may be filled by several tools at once (only
    /// Infrastructure).
    pub fn multiple(self) -> bool {
        matches!(self, Role::Infrastructure)
    }

    /// The directory this role's component is emitted into.
    pub fn dir(self) -> &'static str {
        match self {
            Role::OnChain => contract::DIR_ON_CHAIN,
            Role::OffChain => contract::DIR_OFF_CHAIN,
            Role::Infrastructure => contract::DIR_INFRA,
            Role::Devnet => contract::DIR_DEVNET,
            Role::FormalMethods => contract::DIR_FORMAL_METHODS,
        }
    }
}

/// Human-readable display: "On-chain", "Off-chain", etc.
impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Role::OnChain => "On-chain",
            Role::OffChain => "Off-chain",
            Role::Infrastructure => "Infrastructure",
            Role::Devnet => "Devnet",
            Role::FormalMethods => "Formal methods",
        })
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown role: '{0}'")]
pub struct UnknownRoleError(pub String);

/// A connection seam: a wire protocol an off-chain tool speaks to reach a chain
/// endpoint, and that a devnet or infra tool serves. See
/// [`crate::registry::compat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seam {
    /// Blockfrost-compatible REST (Yaci Store, Blockfrost, …).
    Blockfrost,
    /// Tx3 Transaction Resolve Protocol (JSON-RPC), served by Dolos/Demeter.
    Trp,
    /// `UTxORPC` (u5c) gRPC.
    U5c,
    /// Ogmios JSON-RPC (half of a Kupmios provider, or standalone).
    Ogmios,
    /// Kupo HTTP (the other half of a Kupmios provider).
    Kupo,
}

impl Seam {
    /// Parse from the kebab-case string used in TOML registry files.
    pub fn from_kebab(s: &str) -> Result<Self, UnknownSeamError> {
        match s {
            "blockfrost" => Ok(Seam::Blockfrost),
            "trp" => Ok(Seam::Trp),
            "u5c" => Ok(Seam::U5c),
            "ogmios" => Ok(Seam::Ogmios),
            "kupo" => Ok(Seam::Kupo),
            _ => Err(UnknownSeamError(s.to_string())),
        }
    }

    /// Human-readable label used in compatibility messages.
    pub fn label(self) -> &'static str {
        match self {
            Seam::Blockfrost => "Blockfrost",
            Seam::Trp => "TRP",
            Seam::U5c => "UTxORPC",
            Seam::Ogmios => "Ogmios",
            Seam::Kupo => "Kupo",
        }
    }
}

impl fmt::Display for Seam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("unknown seam: '{0}'")]
pub struct UnknownSeamError(pub String);

/// Devnet-compatibility metadata from a tool's optional `[compat]` table.
///
/// Off-chain tools declare the seams they `consumes`; devnet/infra tools the
/// seams they `serves`. A `self_contained_devnet` tool (e.g. Tx3) bundles its
/// own devnet, so pairing it with a devnet role is incompatible. The default
/// (no `[compat]` table) imposes no constraints.
#[derive(Debug, Clone, Default)]
pub struct CompatConfig {
    pub consumes: Vec<Seam>,
    pub serves: Vec<Seam>,
    pub self_contained_devnet: bool,
}

/// Per-role configuration for a tool.
#[derive(Debug, Clone, Deserialize)]
pub struct RoleConfig {
    /// Path under `templates/` for this tool-role combination.
    pub template: String,
}

/// Maps a `cardano-up context env` output var (`from`, e.g. `KUPO_URL`) to the
/// contract `.env` key the infra component writes (`to`, e.g. `INDEXER_URL`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvMapping {
    pub from: String,
    pub to: String,
}

/// Config for a tool that fills the infrastructure role. All infra tools share
/// one driver template and aggregate into a single `infra/` component.
#[derive(Debug, Clone, Deserialize)]
pub struct InfraConfig {
    /// The package id passed to `cardano-up install`.
    pub cardano_up_package: String,
    #[serde(default)]
    pub env: Vec<EnvMapping>,
}

/// Identifies a tool's generated output inside a role directory, so `doctor`
/// can recognize the tool in a scanned project.
///
/// Matches when `file` (relative to the role dir) exists and, if `contains` is
/// set, its text contains that substring — which keeps generic filenames such
/// as `package.json` from mislabeling foreign projects.
#[derive(Debug, Clone)]
pub struct DetectSignature {
    pub file: String,
    pub contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub name: String,
    pub url: String,
}

/// A loaded tool definition from the registry.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website: String,
    pub community: Vec<Link>,
    pub languages: Vec<String>,
    /// Dependency ids this tool requires; each has a `registry/deps.toml` entry.
    pub system_deps: Vec<String>,
    pub nix_packages: Vec<String>,
    /// The tool ships its own component-local Nix flake. Its `nix_packages` are
    /// then not added to the top-level shell as nixpkgs attrs (a plain `mkShell`
    /// can't build e.g. Plinth); the root flake pulls in the component's shell
    /// via `inputsFrom` instead.
    pub nix_self_contained: bool,
    /// Signatures `doctor` uses to recognize this tool in a scanned project.
    pub detect: Vec<DetectSignature>,
    pub roles: HashMap<Role, RoleConfig>,
    /// Required when the tool fills the infrastructure role (validated at load).
    pub infra: Option<InfraConfig>,
    /// Not production-ready (the upstream tool or its template). It still
    /// generates, but selecting it requires explicit opt-in.
    pub experimental: bool,
    /// Fused on-chain + off-chain template, emitted as one `protocol/`
    /// component. `Some` implies the tool fills both roles (validated at load).
    pub fullstack: Option<RoleConfig>,
    pub compat: CompatConfig,
}

/// One tool assigned to one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAssignment {
    pub role: Role,
    pub tool_id: String,
}

/// Target Cardano network. The scaffolder always emits [`Network::Preview`];
/// the other variants are the values a generated `.env` can be switched to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Preview,
    Preprod,
    Mainnet,
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Network::Preview => "preview",
            Network::Preprod => "preprod",
            Network::Mainnet => "mainnet",
        })
    }
}

impl std::str::FromStr for Network {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "preview" => Ok(Network::Preview),
            "preprod" => Ok(Network::Preprod),
            "mainnet" => Ok(Network::Mainnet),
            _ => Err(()),
        }
    }
}

/// The complete, fully resolved user selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub project_name: String,
    pub assignments: Vec<RoleAssignment>,
    pub network: Network,
    pub nix: bool,
    /// Generate the `spec/` folder (on by default; `--no-spec` turns it off).
    pub spec: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_from_kebab_valid() {
        assert_eq!(Role::from_kebab("on-chain").unwrap(), Role::OnChain);
        assert_eq!(Role::from_kebab("off-chain").unwrap(), Role::OffChain);
        assert_eq!(
            Role::from_kebab("infrastructure").unwrap(),
            Role::Infrastructure
        );
        assert_eq!(Role::from_kebab("devnet").unwrap(), Role::Devnet);
        assert_eq!(
            Role::from_kebab("formal-methods").unwrap(),
            Role::FormalMethods
        );
    }

    #[test]
    fn role_from_kebab_invalid() {
        assert!(Role::from_kebab("onchain").is_err());
        assert!(Role::from_kebab("").is_err());
        assert!(Role::from_kebab("build").is_err());
    }

    #[test]
    fn role_kebab_round_trip() {
        for role in Role::ALL {
            let kebab = role.as_kebab();
            let parsed = Role::from_kebab(kebab).unwrap();
            assert_eq!(*role, parsed);
        }
    }

    #[test]
    fn role_dir_matches_contract() {
        assert_eq!(Role::OnChain.dir(), "on-chain");
        assert_eq!(Role::OffChain.dir(), "off-chain");
        assert_eq!(Role::Infrastructure.dir(), "infra");
        assert_eq!(Role::Devnet.dir(), "devnet");
        assert_eq!(Role::FormalMethods.dir(), "formal-methods");
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::OnChain.to_string(), "On-chain");
        assert_eq!(Role::OffChain.to_string(), "Off-chain");
        assert_eq!(Role::Infrastructure.to_string(), "Infrastructure");
        assert_eq!(Role::Devnet.to_string(), "Devnet");
        assert_eq!(Role::FormalMethods.to_string(), "Formal methods");
    }

    #[test]
    fn role_all_has_five_variants() {
        assert_eq!(Role::ALL.len(), 5);
    }

    #[test]
    fn network_display_matches_env_vocabulary() {
        // These strings are the valid `CARDANO_NETWORK` values in a generated
        // `.env`; Display must render them exactly (templates match on them).
        assert_eq!(Network::Preview.to_string(), "preview");
        assert_eq!(Network::Preprod.to_string(), "preprod");
        assert_eq!(Network::Mainnet.to_string(), "mainnet");
    }
}

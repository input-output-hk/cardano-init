//! The interface contract shared by all templates: canonical paths, component
//! directories, and the standard `.env` variable names.

/// Where on-chain builds write the CIP-57 blueprint, relative to the project root.
pub const BLUEPRINT_PATH: &str = "blueprint/plutus.json";

// Role directories: each role's template is emitted into its directory.
pub const DIR_ON_CHAIN: &str = "on-chain";
pub const DIR_OFF_CHAIN: &str = "off-chain";
pub const DIR_INFRA: &str = "infra";
pub const DIR_DEVNET: &str = "devnet";
pub const DIR_FORMAL_METHODS: &str = "formal-methods";
/// Directory of the fused on-chain + off-chain component emitted from a tool's
/// `[fullstack]` template. Not a role: it composes like an on-chain producer
/// (its `build` writes [`BLUEPRINT_PATH`]).
pub const DIR_PROTOCOL: &str = "protocol";

// Standard `.env` variables. Whichever component provisions an endpoint writes
// its variable; consumers read it and degrade gracefully when it is blank.
pub const ENV_INDEXER_URL: &str = "INDEXER_URL";
pub const ENV_INDEXER_PORT: &str = "INDEXER_PORT";
pub const ENV_NODE_SOCKET_PATH: &str = "NODE_SOCKET_PATH";
pub const ENV_NETWORK: &str = "CARDANO_NETWORK";
/// Ogmios endpoint; seeded empty in every project's `.env`.
pub const ENV_OGMIOS_URL: &str = "OGMIOS_URL";
/// tx-submit-api endpoint; seeded empty.
pub const ENV_TX_SUBMIT_URL: &str = "TX_SUBMIT_URL";
/// Dolos gRPC (`UTxO` RPC) endpoint; seeded empty.
pub const ENV_DOLOS_GRPC_URL: &str = "DOLOS_GRPC_URL";
/// cardano-node-api gRPC endpoint; seeded empty.
pub const ENV_CARDANO_NODE_API_URL: &str = "CARDANO_NODE_API_URL";

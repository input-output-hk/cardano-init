/// Canonical path where on-chain builds produce the CIP-57 blueprint.
/// Relative to project root.
pub const BLUEPRINT_PATH: &str = "blueprint/plutus.json";

/// Directory names for each role. The role template is emitted into this directory.
pub const DIR_ON_CHAIN: &str = "on-chain";
pub const DIR_OFF_CHAIN: &str = "off-chain";
pub const DIR_INFRA: &str = "infra";
pub const DIR_DEVNET: &str = "devnet";
pub const DIR_FORMAL_METHODS: &str = "formal-methods";

/// Standard environment variable names for infrastructure.
/// Infra templates write these to .env; consumers read them.
pub const ENV_INDEXER_URL: &str = "INDEXER_URL";
pub const ENV_INDEXER_PORT: &str = "INDEXER_PORT";
pub const ENV_NODE_SOCKET_PATH: &str = "NODE_SOCKET_PATH";
pub const ENV_NETWORK: &str = "CARDANO_NETWORK";
/// Ogmios WebSocket/HTTP endpoint. Seeded empty in every project's `.env` and
/// populated by the infrastructure component when Ogmios is provisioned; off-chain
/// consumers read it by presence (better to always have the key, even if blank).
pub const ENV_OGMIOS_URL: &str = "OGMIOS_URL";
/// Transaction submission endpoint (tx-submit-api). Seeded empty; populated when
/// the tx-submit-api infrastructure provider is provisioned.
pub const ENV_TX_SUBMIT_URL: &str = "TX_SUBMIT_URL";
/// Dolos gRPC (UTxO RPC) endpoint. Seeded empty; populated when the Dolos
/// infrastructure provider is provisioned.
pub const ENV_DOLOS_GRPC_URL: &str = "DOLOS_GRPC_URL";
/// cardano-node-api gRPC endpoint. Seeded empty; populated when the
/// cardano-node-api infrastructure provider is provisioned.
pub const ENV_CARDANO_NODE_API_URL: &str = "CARDANO_NODE_API_URL";

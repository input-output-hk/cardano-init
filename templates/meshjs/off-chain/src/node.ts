import { existsSync } from "node:fs";

import { config as loadDotenv } from "dotenv";
import { BlockfrostProvider, MeshWallet, YaciProvider } from "@meshsdk/core";

import { GiftCardContract } from "./contract.js";

// Node-only helpers: reading configuration from the environment and wiring up a
// provider + wallet. Importing this in a browser bundle would pull in `node:fs`
// (via dotenv); frontends import the package root ("." → ./contract), which
// already carries the bundled blueprint, and build their own BrowserWallet.

const NETWORK_IDS = { preview: 0, preprod: 0, mainnet: 1 } as const;
export type Network = keyof typeof NETWORK_IDS;

// How this project reaches the chain. The choice is driven entirely by the
// shared ../.env, which is the cardano-init interface contract's connection
// seam — this code never names the tool that wrote it:
//
//   • INDEXER_URL set → a local devnet is up (e.g. Yaci DevKit). Use a
//     YaciProvider against its Blockfrost-compatible API; no Blockfrost project
//     id is needed, and the devnet's faucet (YACI_ADMIN_URL) can fund wallets.
//   • INDEXER_URL absent → talk to a public provider (Blockfrost), which needs
//     BLOCKFROST_PROJECT_ID.
export type ChainProvider = BlockfrostProvider | YaciProvider;

export type ProviderConfig =
  | { kind: "yaci"; url: string; adminUrl?: string }
  | { kind: "blockfrost"; projectId: string };

export type GiftCardEnv = {
  network: Network;
  networkId: 0 | 1;
  provider: ProviderConfig;
  mnemonic: string[];
};

export type EnvResult =
  | { ok: true; env: GiftCardEnv }
  | { ok: false; missing: string[] };

/**
 * Load configuration from the environment. Reads the given dotenv files (the
 * shared `../.env` for CARDANO_NETWORK + the infrastructure connection details,
 * and the gitignored `.env.local` for secrets) without overriding anything
 * already set in `process.env`.
 *
 * Connecting to a local devnet (INDEXER_URL set): requires MNEMONIC.
 * Connecting to Blockfrost (INDEXER_URL absent): requires BLOCKFROST_PROJECT_ID
 * and MNEMONIC. CARDANO_NETWORK is optional (preview | preprod | mainnet;
 * default preview).
 */
export function loadEnv(
  envFiles = ["../.env", ".env.local", ".env"],
): EnvResult {
  for (const path of envFiles) {
    if (existsSync(path)) loadDotenv({ path, override: false });
  }

  const network = (process.env.CARDANO_NETWORK ?? "preview") as Network;
  const mnemonicRaw = process.env.MNEMONIC ?? "";
  const indexerUrl = (process.env.INDEXER_URL ?? "").trim();

  const missing: string[] = [];
  if (!(network in NETWORK_IDS))
    missing.push("CARDANO_NETWORK (one of preview|preprod|mainnet)");

  // The connection seam: a local devnet (INDEXER_URL) wins over Blockfrost.
  let provider: ProviderConfig;
  if (indexerUrl) {
    const adminUrl = (process.env.YACI_ADMIN_URL ?? "").trim();
    provider = { kind: "yaci", url: indexerUrl, adminUrl: adminUrl || undefined };
  } else {
    const projectId = process.env.BLOCKFROST_PROJECT_ID ?? "";
    if (!projectId)
      missing.push("BLOCKFROST_PROJECT_ID (or start a local devnet: INDEXER_URL)");
    provider = { kind: "blockfrost", projectId };
  }

  // A signing wallet is needed in both modes.
  if (!mnemonicRaw.trim()) missing.push("MNEMONIC");
  if (missing.length > 0) return { ok: false, missing };

  return {
    ok: true,
    env: {
      network,
      networkId: NETWORK_IDS[network],
      provider,
      mnemonic: mnemonicRaw.trim().split(/\s+/),
    },
  };
}

/** Construct the chain provider described by the resolved environment. */
export function createProvider(config: ProviderConfig): ChainProvider {
  if (config.kind === "yaci") {
    // Two-arg form wires up the faucet/topup admin API when available.
    return config.adminUrl
      ? new YaciProvider(config.url, config.adminUrl)
      : new YaciProvider(config.url);
  }
  return new BlockfrostProvider(config.projectId);
}

/**
 * Wire up a provider (Yaci devnet or Blockfrost, per the environment), a
 * mnemonic-backed MeshWallet, and a GiftCardContract. The contract uses the
 * blueprint bundled at build time. The wallet is initialized and ready to use.
 * Backend convenience; a frontend builds its own BrowserWallet and constructs
 * GiftCardContract directly.
 */
export async function createGiftCardContractFromEnv(options: {
  env: GiftCardEnv;
}): Promise<{
  contract: GiftCardContract;
  wallet: MeshWallet;
  provider: ChainProvider;
}> {
  const { env } = options;
  const provider = createProvider(env.provider);
  const wallet = new MeshWallet({
    networkId: env.networkId,
    fetcher: provider,
    submitter: provider,
    key: { type: "mnemonic", words: env.mnemonic },
  });
  await wallet.init();

  const contract = new GiftCardContract({
    fetcher: provider,
    submitter: provider,
    wallet,
    networkId: env.networkId,
  });

  return { contract, wallet, provider };
}

/**
 * On a local Yaci devnet, fund an address from the built-in faucet so a fresh
 * wallet has something to spend. No-op on Blockfrost (use a pre-funded wallet
 * there). Requires YACI_ADMIN_URL to have been published to ../.env by the
 * devnet's `just dev`.
 *
 * `ada` is the amount in **ADA** (Yaci's topup unit), not lovelace.
 */
export async function topupOnDevnet(
  provider: ChainProvider,
  address: string,
  ada: string,
): Promise<boolean> {
  if (provider instanceof YaciProvider) {
    await provider.addressTopup(address, ada);
    return true;
  }
  return false;
}

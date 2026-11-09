import { existsSync } from "node:fs";

import { config as loadDotenv } from "dotenv";
import {
  Blockfrost,
  Kupmios,
  Lucid,
  type LucidEvolution,
  type Network as LucidNetwork,
  type Provider,
  type SlotConfig,
} from "@lucid-evolution/lucid";

import { GiftCardContract } from "./contract.js";

// Node-only helpers: reading configuration from the environment and wiring up a
// provider + wallet into a Lucid Evolution instance. Importing this in a
// browser bundle would pull in `node:fs` (via dotenv); frontends import the
// package root ("." → ./contract), which already carries the bundled blueprint,
// and build their own Lucid instance with a provider, then
// `lucid.selectWallet.fromAPI(walletApi)`.

const LUCID_NETWORKS = {
  preview: "Preview",
  preprod: "Preprod",
  mainnet: "Mainnet",
} as const satisfies Record<string, LucidNetwork>;
export type Network = keyof typeof LUCID_NETWORKS;

const BLOCKFROST_BASE_URLS: Record<Network, string> = {
  preview: "https://cardano-preview.blockfrost.io/api/v0",
  preprod: "https://cardano-preprod.blockfrost.io/api/v0",
  mainnet: "https://cardano-mainnet.blockfrost.io/api/v0",
};

// How this project reaches the chain. The choice is driven entirely by the
// shared ../.env, which is the cardano-init interface contract's connection
// seam — this code never names the tool that wrote it:
//
//   • INDEXER_URL + YACI_ADMIN_URL → a local Yaci devnet. Yaci serves a
//     Blockfrost-compatible API; its admin API supplies the devnet's slot
//     config and a faucet that funds wallets.
//   • INDEXER_URL + OGMIOS_URL → Kupo (INDEXER_URL) + Ogmios: a Kupmios provider.
//   • INDEXER_URL alone → a local Blockfrost-compatible endpoint (e.g. Dingo).
//   • INDEXER_URL absent → public Blockfrost, which needs BLOCKFROST_PROJECT_ID.
export type ProviderConfig =
  | { kind: "yaci"; baseUrl: string; adminUrl: string }
  | { kind: "kupmios"; kupoUrl: string; ogmiosUrl: string }
  | { kind: "blockfrost"; baseUrl: string; projectId: string };

export type GiftCardEnv = {
  network: Network;
  provider: ProviderConfig;
  mnemonic: string;
};

export type EnvResult =
  | { ok: true; env: GiftCardEnv }
  | { ok: false; missing: string[] };

// Strip any trailing slash: Blockfrost-style providers join paths as
// `${baseUrl}/addresses/…`, so a trailing slash (Yaci publishes INDEXER_URL as
// `…/api/v1/`) would produce a `//` that the indexer 404s.
const trimSlash = (url: string) => url.replace(/\/+$/, "");

/**
 * Load configuration from the environment. Reads the given dotenv files (the
 * shared `../.env` for CARDANO_NETWORK + the connection details, and the
 * gitignored `.env.local` for secrets) without overriding anything already set
 * in `process.env`.
 *
 * Connecting to a local endpoint (INDEXER_URL set): requires MNEMONIC.
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
  const mnemonic = (process.env.MNEMONIC ?? "").trim();
  const indexerUrl = trimSlash((process.env.INDEXER_URL ?? "").trim());
  const adminUrl = trimSlash((process.env.YACI_ADMIN_URL ?? "").trim());
  const ogmiosUrl = trimSlash((process.env.OGMIOS_URL ?? "").trim());

  const missing: string[] = [];
  if (!(network in LUCID_NETWORKS))
    missing.push("CARDANO_NETWORK (one of preview|preprod|mainnet)");

  // The connection seam: a local endpoint (INDEXER_URL) wins over Blockfrost.
  let provider: ProviderConfig;
  if (indexerUrl && adminUrl) {
    provider = { kind: "yaci", baseUrl: indexerUrl, adminUrl };
  } else if (indexerUrl && ogmiosUrl) {
    provider = { kind: "kupmios", kupoUrl: indexerUrl, ogmiosUrl };
  } else if (indexerUrl) {
    // A local Blockfrost-compatible API ignores the project id.
    provider = { kind: "blockfrost", baseUrl: indexerUrl, projectId: "local" };
  } else {
    const projectId = process.env.BLOCKFROST_PROJECT_ID ?? "";
    if (!projectId)
      missing.push("BLOCKFROST_PROJECT_ID (or start a local devnet: INDEXER_URL)");
    provider = {
      kind: "blockfrost",
      baseUrl: BLOCKFROST_BASE_URLS[network in LUCID_NETWORKS ? network : "preview"],
      projectId,
    };
  }

  // A signing wallet is needed in every mode.
  if (!mnemonic) missing.push("MNEMONIC");
  if (missing.length > 0) return { ok: false, missing };

  return { ok: true, env: { network, provider, mnemonic } };
}

/**
 * Read Yaci DevKit's live shelley genesis and turn it into a Lucid
 * `SlotConfig`, so slot ↔ time conversion matches the running devnet (which
 * resets whenever the cluster restarts).
 */
export async function fetchYaciSlotConfig(adminUrl: string): Promise<SlotConfig> {
  const res = await fetch(
    `${trimSlash(adminUrl)}/local-cluster/api/admin/devnet/genesis/shelley`,
  );
  if (!res.ok) throw new Error(`Fetching devnet genesis failed: HTTP ${res.status}`);
  const shelley = (await res.json()) as { systemStart: string; slotLength: number };
  return {
    zeroTime: new Date(shelley.systemStart).getTime(),
    zeroSlot: 0,
    slotLength: shelley.slotLength * 1000, // seconds → milliseconds
  };
}

/**
 * Build a Lucid Evolution instance connected to the configured provider, with
 * no wallet selected. Pick one with `selectWallet.fromAddress(address, utxos)`
 * (read-only, e.g. a backend building for the user's wallet) or
 * `selectWallet.fromSeed(...)` (signing, see `createLucid`).
 *
 * The Yaci path runs on Lucid's "Custom" network with the devnet's live slot
 * config; every other path uses the named network from CARDANO_NETWORK.
 */
export async function createProviderLucid(
  env: Pick<GiftCardEnv, "network" | "provider">,
): Promise<LucidEvolution> {
  const { provider } = env;
  let p: Provider;
  switch (provider.kind) {
    case "yaci":
      p = new Blockfrost(trimSlash(provider.baseUrl), "yaci-devnet");
      return Lucid(p, "Custom", {
        slotConfig: await fetchYaciSlotConfig(provider.adminUrl),
      });
    case "kupmios":
      p = new Kupmios(trimSlash(provider.kupoUrl), trimSlash(provider.ogmiosUrl));
      break;
    case "blockfrost":
      p = new Blockfrost(trimSlash(provider.baseUrl), provider.projectId);
      break;
  }
  return Lucid(p, LUCID_NETWORKS[env.network]);
}

/** Build a Lucid Evolution instance with the MNEMONIC wallet selected. */
export async function createLucid(env: GiftCardEnv): Promise<LucidEvolution> {
  const lucid = await createProviderLucid(env);
  lucid.selectWallet.fromSeed(env.mnemonic);
  return lucid;
}

/**
 * Wire up a provider (per the environment), a mnemonic-backed wallet, and a
 * GiftCardContract. The contract uses the blueprint bundled at build time.
 * Backend convenience; a frontend builds its own Lucid instance (a provider,
 * then `selectWallet.fromAPI(...)`) and constructs GiftCardContract directly.
 */
export async function createGiftCardContractFromEnv(options: {
  env: GiftCardEnv;
}): Promise<{
  contract: GiftCardContract;
  lucid: LucidEvolution;
  provider: ProviderConfig;
}> {
  const { env } = options;
  const lucid = await createLucid(env);
  const contract = new GiftCardContract({ lucid });
  return { contract, lucid, provider: env.provider };
}

/** The selected wallet's own bech32 address. */
export async function walletAddress(lucid: LucidEvolution): Promise<string> {
  return lucid.wallet().address();
}

/**
 * On a local Yaci devnet, fund an address from the built-in faucet so a fresh
 * wallet has something to spend. No-op on any other provider (use a pre-funded
 * wallet there). `ada` is the amount in **ADA** (Yaci's topup unit).
 */
export async function topupOnDevnet(
  provider: ProviderConfig,
  address: string,
  ada: number,
): Promise<boolean> {
  if (provider.kind !== "yaci") return false;
  const res = await fetch(`${trimSlash(provider.adminUrl)}/local-cluster/api/addresses/topup`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ address, adaAmount: ada }),
  });
  if (!res.ok) {
    throw new Error(`Devnet faucet topup failed: HTTP ${res.status}`);
  }
  return true;
}

import { existsSync } from "node:fs";

import { config as loadDotenv } from "dotenv";
import { Unwrapped } from "@blaze-cardano/ogmios";
import {
  Blaze,
  Blockfrost,
  ColdWallet,
  Core,
  HotWallet,
  Kupmios,
  type NetworkName,
  type Provider,
  type Wallet,
} from "@blaze-cardano/sdk";

import { GiftCardContract } from "./contract.js";

// Node-only helpers: reading configuration from the environment and wiring up a
// provider + wallet into a Blaze instance. Importing this in a browser bundle
// would pull in `node:fs` (via dotenv); frontends import the package root
// ("." → ./contract), which already carries the bundled blueprint, and build
// their own Blaze instance from a provider and a `WebWallet` (CIP-30).

const BLAZE_NETWORKS = {
  preview: "cardano-preview",
  preprod: "cardano-preprod",
  mainnet: "cardano-mainnet",
} as const satisfies Record<string, NetworkName>;
export type Network = keyof typeof BLAZE_NETWORKS;

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
  | { kind: "blockfrost"; baseUrl?: string; projectId: string };

export type GiftCardEnv = {
  network: Network;
  provider: ProviderConfig;
  mnemonic: string;
};

export type EnvResult =
  | { ok: true; env: GiftCardEnv }
  | { ok: false; missing: string[] };

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
  if (!(network in BLAZE_NETWORKS))
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
    provider = { kind: "blockfrost", projectId };
  }

  // A signing wallet is needed in every mode.
  if (!mnemonic) missing.push("MNEMONIC");
  if (missing.length > 0) return { ok: false, missing };

  return { ok: true, env: { network, provider, mnemonic } };
}

/**
 * Blaze's `Blockfrost` provider always targets blockfrost.io. This subclass
 * points it at any Blockfrost-compatible base URL (Yaci Store, Dingo, …), and
 * optionally overrides the slot config (a devnet has its own genesis).
 *
 * Blaze joins every path as `${url}/path` except the protocol parameters
 * (`${url}epochs/…`), relying on blockfrost.io tolerating `//`. Yaci 404s on
 * `//`, so `url` is kept slash-free and only `getParameters` goes through a
 * helper provider whose URL ends in a slash: no `//` ever reaches the server.
 */
export class LocalBlockfrost extends Blockfrost {
  private readonly slotConfig?: Core.SlotConfig;
  private readonly paramsProvider: Blockfrost;

  constructor(input: {
    baseUrl: string;
    network: NetworkName;
    slotConfig?: Core.SlotConfig;
  }) {
    super({ network: input.network, projectId: "local" });
    this.url = trimSlash(input.baseUrl);
    this.slotConfig = input.slotConfig;
    this.paramsProvider = new Blockfrost({ network: input.network, projectId: "local" });
    this.paramsProvider.url = `${this.url}/`;
  }

  override getSlotConfig(): Core.SlotConfig {
    return this.slotConfig ?? super.getSlotConfig();
  }

  override getParameters(): Promise<Core.ProtocolParameters> {
    return this.paramsProvider.getParameters();
  }
}

/**
 * Read Yaci DevKit's live shelley genesis and turn it into a Blaze
 * `SlotConfig`, so slot ↔ time conversion matches the running devnet (which
 * resets whenever the cluster restarts).
 */
export async function fetchYaciSlotConfig(adminUrl: string): Promise<Core.SlotConfig> {
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

/** Ogmios speaks JSON-RPC over WebSocket; accept an http(s):// URL too. */
const toWebSocketUrl = (url: string) => url.replace(/^http(s?):\/\//, "ws$1://");

/** Build the Blaze provider described by the environment. */
export async function createProvider(
  env: Pick<GiftCardEnv, "network" | "provider">,
): Promise<Provider> {
  const { provider } = env;
  const network = BLAZE_NETWORKS[env.network];
  switch (provider.kind) {
    case "yaci":
      return new LocalBlockfrost({
        baseUrl: provider.baseUrl,
        network,
        slotConfig: await fetchYaciSlotConfig(provider.adminUrl),
      });
    case "kupmios":
      return new Kupmios(
        trimSlash(provider.kupoUrl),
        await Unwrapped.Ogmios.new(toWebSocketUrl(provider.ogmiosUrl)),
      );
    case "blockfrost":
      return provider.baseUrl
        ? new LocalBlockfrost({ baseUrl: provider.baseUrl, network })
        : new Blockfrost({ network, projectId: provider.projectId });
  }
}

/** Derive a Blaze `HotWallet` (signing) from a BIP-39 mnemonic. */
export async function walletFromMnemonic(
  mnemonic: string,
  provider: Provider,
): Promise<HotWallet> {
  const entropy = Core.mnemonicToEntropy(mnemonic, Core.wordlist);
  const masterkey = Core.Bip32PrivateKey.fromBip39Entropy(Buffer.from(entropy), "");
  return HotWallet.fromMasterkey(masterkey.hex(), provider);
}

/**
 * A read-only Blaze instance for building transactions on behalf of another
 * address (e.g. a backend building for the user's wallet, which then signs).
 */
export async function createReadOnlyBlaze(
  env: Pick<GiftCardEnv, "network" | "provider">,
  address: string,
): Promise<Blaze<Provider, ColdWallet>> {
  const provider = await createProvider(env);
  const wallet = new ColdWallet(Core.addressFromBech32(address), provider.network, provider);
  return Blaze.from(provider, wallet);
}

/** Build a Blaze instance with the MNEMONIC wallet. */
export async function createBlaze(env: GiftCardEnv): Promise<Blaze<Provider, HotWallet>> {
  const provider = await createProvider(env);
  return Blaze.from(provider, await walletFromMnemonic(env.mnemonic, provider));
}

/**
 * Wire up a provider (per the environment), a mnemonic-backed wallet, and a
 * GiftCardContract. The contract uses the blueprint bundled at build time.
 * Backend convenience; a frontend builds its own Blaze instance (a provider +
 * a `WebWallet`) and constructs GiftCardContract directly.
 */
export async function createGiftCardContractFromEnv(options: {
  env: GiftCardEnv;
}): Promise<{
  contract: GiftCardContract;
  blaze: Blaze<Provider, HotWallet>;
  provider: ProviderConfig;
}> {
  const { env } = options;
  const blaze = await createBlaze(env);
  const contract = new GiftCardContract({ blaze });
  return { contract, blaze, provider: env.provider };
}

/** The wallet's own bech32 address. */
export async function walletAddress(blaze: Blaze<Provider, Wallet>): Promise<string> {
  return (await blaze.wallet.getChangeAddress()).toBech32();
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

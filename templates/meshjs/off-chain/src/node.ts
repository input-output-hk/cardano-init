import { existsSync } from "node:fs";

import { config as loadDotenv } from "dotenv";
import { BlockfrostProvider, MeshWallet } from "@meshsdk/core";

import { GiftCardContract } from "./contract.js";

// Node-only helpers: reading configuration from the environment and wiring up a
// provider + wallet. Importing this in a browser bundle would pull in `node:fs`
// (via dotenv); frontends import the package root ("." → ./contract), which
// already carries the bundled blueprint, and build their own BrowserWallet.

const NETWORK_IDS = { preview: 0, preprod: 0, mainnet: 1 } as const;
export type Network = keyof typeof NETWORK_IDS;

export type GiftCardEnv = {
  network: Network;
  networkId: 0 | 1;
  blockfrostProjectId: string;
  mnemonic: string[];
};

export type EnvResult =
  | { ok: true; env: GiftCardEnv }
  | { ok: false; missing: string[] };

/**
 * Load configuration from the environment. Reads the given dotenv files (the
 * shared `../.env` for CARDANO_NETWORK and the gitignored `.env.local` for
 * secrets) without overriding anything already set in `process.env`.
 *
 * Required: BLOCKFROST_PROJECT_ID, MNEMONIC (space-separated words).
 * Optional: CARDANO_NETWORK (preview | preprod | mainnet; default preview).
 */
export function loadEnv(
  envFiles = ["../.env", ".env.local", ".env"],
): EnvResult {
  for (const path of envFiles) {
    if (existsSync(path)) loadDotenv({ path, override: false });
  }

  const network = (process.env.CARDANO_NETWORK ?? "preview") as Network;
  const blockfrostProjectId = process.env.BLOCKFROST_PROJECT_ID ?? "";
  const mnemonicRaw = process.env.MNEMONIC ?? "";

  const missing: string[] = [];
  if (!(network in NETWORK_IDS))
    missing.push("CARDANO_NETWORK (one of preview|preprod|mainnet)");
  if (!blockfrostProjectId) missing.push("BLOCKFROST_PROJECT_ID");
  if (!mnemonicRaw.trim()) missing.push("MNEMONIC");
  if (missing.length > 0) return { ok: false, missing };

  return {
    ok: true,
    env: {
      network,
      networkId: NETWORK_IDS[network],
      blockfrostProjectId,
      mnemonic: mnemonicRaw.trim().split(/\s+/),
    },
  };
}

/**
 * Wire up a BlockfrostProvider, a mnemonic-backed MeshWallet, and a
 * GiftCardContract from the environment. The contract uses the blueprint
 * bundled at build time. The wallet is initialized and ready to use. Backend
 * convenience; a frontend builds its own BrowserWallet and constructs
 * GiftCardContract directly.
 */
export async function createGiftCardContractFromEnv(options: {
  env: GiftCardEnv;
}): Promise<{
  contract: GiftCardContract;
  wallet: MeshWallet;
  provider: BlockfrostProvider;
}> {
  const { env } = options;
  const provider = new BlockfrostProvider(env.blockfrostProjectId);
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

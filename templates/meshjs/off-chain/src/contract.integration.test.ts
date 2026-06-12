import { existsSync } from "node:fs";

import { config as loadDotenv } from "dotenv";
import { MeshWallet, YaciProvider } from "@meshsdk/core";
import { describe, expect, it } from "vitest";

import { GiftCardContract, hasBundledBlueprint } from "./contract.js";

// End-to-end integration test: a full mint→lock→redeem round-trip against a
// real local devnet (Yaci DevKit). It is gated so it runs ONLY when a devnet is
// available and the on-chain blueprint has been built:
//
//   • INDEXER_URL — written to ../.env by `just -f test/Justfile dev`, or
//     exported by the testing component's ephemeral `just test`.
//   • a bundled blueprint — produced by the on-chain `just build`.
//
// Otherwise it skips, so `just test` stays green with no devnet. This file ends
// in `.test.ts`, so it is excluded from the library build (tsconfig) and never
// reaches the importable package — only the unit-tested `contract.ts` does.

for (const path of ["../.env", ".env.local", ".env"]) {
  if (existsSync(path)) loadDotenv({ path, override: false });
}

const indexerUrl = (process.env.INDEXER_URL ?? "").trim();
const adminUrl = (process.env.YACI_ADMIN_URL ?? "http://localhost:10000").trim();
const canRun = indexerUrl !== "" && hasBundledBlueprint();

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Poll `fn` until `ok(result)` or we run out of tries. */
async function poll<T>(
  fn: () => Promise<T>,
  ok: (v: T) => boolean,
  tries = 60,
  ms = 1000,
): Promise<T> {
  let last!: T;
  for (let i = 0; i < tries; i++) {
    last = await fn();
    if (ok(last)) return last;
    await wait(ms);
  }
  throw new Error("timed out waiting for the devnet to reach the expected state");
}

/**
 * Wait until a submitted tx is on-chain (its outputs are queryable). The
 * indexer returns 404 for a not-yet-included tx, so tolerate errors and retry.
 */
async function confirmed(
  provider: YaciProvider,
  txHash: string,
  tries = 60,
  ms = 1000,
): Promise<void> {
  for (let i = 0; i < tries; i++) {
    try {
      const utxos = await provider.fetchUTxOs(txHash);
      if (utxos.length > 0) return;
    } catch {
      // not indexed yet — keep waiting
    }
    await wait(ms);
  }
  throw new Error(`tx ${txHash} was not confirmed on the devnet in time`);
}

(canRun ? describe : describe.skip)("GiftCard round-trip on a Yaci devnet", () => {
  it("mints + locks a gift card, then redeems it", async () => {
    const provider = new YaciProvider(indexerUrl, adminUrl);

    // A fresh throwaway wallet — no seed phrase needed, the devnet faucet funds
    // it. (brew() returns the mnemonic words; tolerate string or string[].)
    const brewed = MeshWallet.brew();
    const words = Array.isArray(brewed) ? brewed : String(brewed).split(" ");
    const wallet = new MeshWallet({
      networkId: 0,
      fetcher: provider,
      submitter: provider,
      key: { type: "mnemonic", words },
    });
    await wallet.init();
    const address = await wallet.getChangeAddress();

    // Fund from the faucet: a small UTxO usable as collateral + a large one to
    // spend. NOTE: Yaci's topup amount is in ADA, not lovelace. Wait until both
    // UTxOs are indexed before building a transaction.
    await provider.addressTopup(address, "10"); // 10 ADA (collateral)
    await provider.addressTopup(address, "10000"); // 10,000 ADA (funds)
    await poll(
      () => provider.fetchAddressUTxOs(address),
      (utxos) => utxos.length >= 2,
    );

    const contract = new GiftCardContract({
      fetcher: provider,
      submitter: provider,
      wallet,
      networkId: 0,
    });

    // Create: mint a unique token and lock 5 ADA at the redeem script address.
    const createTx = await contract.signAndSubmit(
      await contract.createGiftCard("IntegrationGift", [
        { unit: "lovelace", quantity: "5000000" },
      ]),
    );
    await confirmed(provider, createTx);

    const giftUtxo = await contract.getGiftCardUtxo(createTx);
    expect(giftUtxo, "the create tx should produce a gift-card UTxO").toBeDefined();

    // Redeem: burn the token and release the locked assets back to the wallet.
    const redeemTx = await contract.signAndSubmit(
      await contract.redeemGiftCard(giftUtxo!),
    );
    await confirmed(provider, redeemTx);
    expect(redeemTx).toMatch(/^[0-9a-f]{64}$/);
  }, 180_000);
});

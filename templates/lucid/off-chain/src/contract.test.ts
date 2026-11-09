import {
  Emulator,
  generateEmulatorAccount,
  getAddressDetails,
  Lucid,
  type LucidEvolution,
} from "@lucid-evolution/lucid";
import { beforeAll, describe, expect, it } from "vitest";

import {
  getBundledValidators,
  GiftCardContract,
  hasBundledBlueprint,
} from "./contract.js";

// The blueprint is bundled before tests run (the npm "pretest" hook calls
// scripts/bundle-blueprint.mjs), so these run only once the on-chain component
// has been built. They need no wallet funds or network: Lucid Evolution's
// in-memory `Emulator` stands in for the chain and evaluates the scripts
// locally, so the full mint → lock → redeem flow runs against the real
// validators without a devnet.
const bundled = hasBundledBlueprint();

// A throwaway seed UTxO — script derivation is pure, so any well-formed
// reference works for an offline test.
const SEED_UTXO = {
  txHash: "0000000000000000000000000000000000000000000000000000000000000000",
  outputIndex: 0,
};

describe.runIf(bundled)("GiftCard off-chain (Lucid Evolution)", () => {
  let emulator: Emulator;
  let lucid: LucidEvolution;

  beforeAll(async () => {
    const account = generateEmulatorAccount({ lovelace: 1_000_000_000n });
    emulator = new Emulator([account]);
    lucid = await Lucid(emulator, "Custom");
    lucid.selectWallet.fromSeed(account.seedPhrase);
  });

  it("bundles both validators from the blueprint", () => {
    const { giftCard, redeem } = getBundledValidators();
    expect(giftCard.length).toBeGreaterThan(0);
    expect(redeem.length).toBeGreaterThan(0);
  });

  it("derives a policy id and redeem address from the bundled blueprint", () => {
    const contract = new GiftCardContract({ lucid });
    const scripts = contract.getScripts("hello world", SEED_UTXO);

    // Policy id is a 28-byte blake2b-224 hash → 56 hex chars.
    expect(scripts.policyId).toMatch(/^[0-9a-f]{56}$/);
    // Testnet script addresses are bech32 with the `addr_test` HRP, and the
    // payment credential is the redeem script itself.
    expect(scripts.redeemAddress.startsWith("addr_test")).toBe(true);
    expect(getAddressDetails(scripts.redeemAddress).paymentCredential?.type).toBe(
      "Script",
    );
    // The asset unit is policy id + hex token name ("hello world").
    expect(scripts.unit).toBe(scripts.policyId + "68656c6c6f20776f726c64");
  });

  it("mints + locks a gift card, then redeems it on the emulator", async () => {
    const contract = new GiftCardContract({ lucid });

    // Create: mint a unique token and lock 5 ADA at the redeem script address.
    const { txCbor, redeemAddress, unit } = await contract.createGiftCard(
      "EmulatorGift",
      5_000_000n,
    );
    await lucid.awaitTx(await contract.signAndSubmit(txCbor));

    const giftUtxo = await contract.getGiftCardUtxoAt(redeemAddress);
    expect(giftUtxo, "the create tx should produce a gift-card UTxO").toBeDefined();
    expect(giftUtxo!.assets[unit]).toBe(1n);

    // Redeem: burn the token and release the locked assets back to the wallet.
    const redeemTx = await contract.signAndSubmit(
      await contract.redeemGiftCard(giftUtxo!),
    );
    await lucid.awaitTx(redeemTx);
    expect(redeemTx).toMatch(/^[0-9a-f]{64}$/);

    // The script address is empty and the token is gone from the wallet.
    expect(await lucid.utxosAt(redeemAddress)).toHaveLength(0);
    const walletUtxos = await lucid.wallet().getUtxos();
    expect(walletUtxos.some((u) => (u.assets[unit] ?? 0n) > 0n)).toBe(false);
  });
});

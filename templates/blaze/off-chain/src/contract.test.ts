import { Emulator } from "@blaze-cardano/emulator";
import { Core } from "@blaze-cardano/sdk";
import { beforeEach, describe, expect, it } from "vitest";

import {
  getBundledValidators,
  GiftCardContract,
  hasBundledBlueprint,
} from "./contract.js";

// The blueprint is bundled before tests run (the npm "pretest" hook calls
// scripts/bundle-blueprint.mjs), so these run only once the on-chain component
// has been built. They need no wallet funds or network: Blaze's in-memory
// `Emulator` stands in for the chain and evaluates the scripts locally, so the
// full mint → lock → redeem flow runs against the real validators without a
// devnet.
const bundled = hasBundledBlueprint();

// A throwaway seed UTxO — script derivation is pure, so any well-formed
// reference works for an offline test.
const SEED_UTXO = {
  txHash: "0000000000000000000000000000000000000000000000000000000000000000",
  outputIndex: 0,
};

const WALLET = "alice";

describe.runIf(bundled)("GiftCard off-chain (Blaze)", () => {
  let emulator: Emulator;

  beforeEach(async () => {
    emulator = new Emulator([]);
    // Two UTxOs: one to spend, one Blaze can pick as collateral.
    await emulator.register(WALLET, new Core.Value(1_000_000_000n));
    await emulator.register(WALLET, new Core.Value(10_000_000n));
  });

  it("bundles both validators from the blueprint", () => {
    const { giftCard, redeem } = getBundledValidators();
    expect(giftCard.length).toBeGreaterThan(0);
    expect(redeem.length).toBeGreaterThan(0);
  });

  it("derives a policy id and redeem address from the bundled blueprint", async () => {
    await emulator.as(WALLET, async (blaze) => {
      const contract = new GiftCardContract({ blaze });
      const scripts = contract.getScripts("hello world", SEED_UTXO);

      // Policy id is a 28-byte blake2b-224 hash → 56 hex chars.
      expect(scripts.policyId).toMatch(/^[0-9a-f]{56}$/);
      // Testnet script addresses are bech32 with the `addr_test` HRP, and the
      // payment credential is the redeem script itself.
      expect(scripts.redeemAddress.startsWith("addr_test")).toBe(true);
      const credential = Core.addressFromBech32(scripts.redeemAddress)
        .getProps()
        .paymentPart;
      expect(credential?.type).toBe(Core.CredentialType.ScriptHash);
      expect(credential?.hash).toBe(scripts.redeemScript.hash());
      // The asset unit is policy id + hex token name ("hello world").
      expect(scripts.unit).toBe(scripts.policyId + "68656c6c6f20776f726c64");
    });
  });

  it("mints + locks a gift card, then redeems it on the emulator", async () => {
    await emulator.as(WALLET, async (blaze, address) => {
      const contract = new GiftCardContract({ blaze });

      // Create: mint a unique token and lock 5 ADA at the redeem script address.
      const { tx, redeemAddress, unit } = await contract.createGiftCard(
        "EmulatorGift",
        5_000_000n,
      );
      const createTx = await contract.signAndSubmit(tx);
      emulator.awaitTransactionConfirmation(Core.TransactionId(createTx));

      const giftUtxo = await contract.getGiftCardUtxoAt(redeemAddress);
      expect(giftUtxo, "the create tx should produce a gift-card UTxO").toBeDefined();
      expect(giftUtxo!.output().amount().multiasset()?.get(Core.AssetId(unit))).toBe(1n);

      // Redeem: burn the token and release the locked assets back to the wallet.
      const redeemTx = await contract.signAndSubmit(
        await contract.redeemGiftCard(giftUtxo!),
      );
      emulator.awaitTransactionConfirmation(Core.TransactionId(redeemTx));
      expect(redeemTx).toMatch(/^[0-9a-f]{64}$/);

      // The script address is empty and the token is gone from the wallet.
      expect(await contract.getGiftCardUtxoAt(redeemAddress)).toBeUndefined();
      const walletUtxos = await blaze.provider.getUnspentOutputs(address);
      expect(
        walletUtxos.some((u) => u.output().amount().multiasset()?.has(Core.AssetId(unit))),
      ).toBe(false);
    });
  });

  it("rejects spending the gift without burning the token", async () => {
    await emulator.as(WALLET, async (blaze) => {
      const contract = new GiftCardContract({ blaze });
      const { tx, redeemAddress, seedUtxo } = await contract.createGiftCard(
        "UnburnedGift",
        5_000_000n,
      );
      emulator.awaitTransactionConfirmation(
        Core.TransactionId(await contract.signAndSubmit(tx)),
      );
      const giftUtxo = await contract.getGiftCardUtxoAt(redeemAddress);
      expect(giftUtxo).toBeDefined();

      // Spend the locked UTxO with the redeem script but WITHOUT burning the
      // token: the validator must reject it (burning is the authorization).
      const { redeemScript } = contract.getScripts("UnburnedGift", seedUtxo);
      const cheat = blaze
        .newTransaction()
        .addInput(giftUtxo!, Core.PlutusData.newConstrPlutusData(
          new Core.ConstrPlutusData(0n, new Core.PlutusList()),
        ))
        .provideScript(redeemScript);
      await emulator.expectScriptFailure(cheat);
    });
  });
});

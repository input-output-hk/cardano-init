import {
  Core,
  cborToScript,
  applyParams,
  type Blaze,
  type Provider,
  type Wallet,
} from "@blaze-cardano/sdk";

import { bundledBlueprint } from "./blueprint.generated.js";

// Framework-agnostic bindings for the GiftCard contract, built on Blaze. No
// Node-only dependencies (no `fs`), so this module is safe to import in a
// browser frontend as well as a backend: the on-chain blueprint is bundled in
// at build time from ../blueprint/plutus.json (see
// scripts/bundle-blueprint.mjs), so the contract always knows its validators —
// consumers never supply them.

/** Plutus language version the on-chain validators are compiled to. */
export const LANGUAGE_VERSION = "PlutusV3" as const;

/**
 * Shape of the CIP-57 blueprint produced by the on-chain `build`. Only the
 * fields this library reads are named; the index signatures tolerate the many
 * other fields a real plutus.json carries (preamble, redeemer, datum, …).
 */
export type Blueprint = {
  validators: { title: string; compiledCode: string; [key: string]: unknown }[];
  [key: string]: unknown;
};

/** The two compiled validators the GiftCard flow needs (blueprint hex). */
export type GiftCardValidators = {
  /** Compiled code of the `gift_card` minting policy. */
  giftCard: string;
  /** Compiled code of the `redeem` spending validator. */
  redeem: string;
};

/** A reference to the seed UTxO that parameterises a gift card. */
export type SeedUtxo = {
  txHash: string;
  outputIndex: number;
};

/** The result of a successful `createGiftCard`. */
export type GiftCardCreation = {
  /** The unsigned create transaction — sign and submit it. */
  tx: Core.Transaction;
  /** The same transaction as CBOR hex, e.g. to hand to a CIP-30 wallet. */
  txCbor: string;
  /** Bech32 redeem script address the gift lands at — pass to `getGiftCardUtxoAt`. */
  redeemAddress: string;
  /** The gift-card token's asset unit (policy id + hex token name). */
  unit: string;
  /** The seed UTxO consumed by the one-shot policy (with the token name, it fixes the scripts). */
  seedUtxo: SeedUtxo;
};

/**
 * The parameterised scripts and derived identifiers for a single gift card.
 * Useful on a frontend to display the policy id / script address without
 * building a transaction.
 */
export type GiftCardScripts = {
  giftCardScript: Core.Script;
  policyId: string;
  redeemScript: Core.Script;
  /** Bech32 address of the redeem script. */
  redeemAddress: string;
  /** The full asset unit (policy id + hex token name) of the gift-card token. */
  unit: string;
};

// On-chain tools prefix validator titles with their module name, e.g.
// "giftcard.gift_card.mint". We match on the "<validator>.<purpose>" suffix so
// the lookup is independent of the module (file) name.
export function findValidator(blueprint: Blueprint, suffix: string): string {
  const validator = blueprint.validators.find(
    (v) => v.title === suffix || v.title.endsWith(`.${suffix}`),
  );
  if (!validator) {
    const known = blueprint.validators.map((v) => v.title).join(", ");
    throw new Error(
      `Validator "${suffix}" not found in blueprint. Available: ${known}`,
    );
  }
  return validator.compiledCode;
}

/** Pull the gift_card (mint) and redeem (spend) validators out of a blueprint. */
export function getGiftCardValidators(blueprint: Blueprint): GiftCardValidators {
  return {
    giftCard: findValidator(blueprint, "gift_card.mint"),
    redeem: findValidator(blueprint, "redeem.spend"),
  };
}

/** The blueprint inlined at build time, or null if none was bundled. */
export { bundledBlueprint };

/** Whether a blueprint was bundled into this build. */
export function hasBundledBlueprint(): boolean {
  return bundledBlueprint !== null;
}

/**
 * The GiftCard validators bundled at build time. Throws if none was bundled —
 * build the on-chain component and run the off-chain `just build`, or pass
 * `validators` to the contract explicitly.
 */
export function getBundledValidators(): GiftCardValidators {
  if (bundledBlueprint === null) {
    throw new Error(
      "No blueprint was bundled into this build. Build the on-chain component " +
        "(its `just build` writes ../blueprint/plutus.json), then run the " +
        "off-chain `just build`; or pass `validators` to GiftCardContract.",
    );
  }
  return getGiftCardValidators(bundledBlueprint);
}

// --- PlutusData helpers -----------------------------------------------------

const bytes = (hex: string) => Core.PlutusData.newBytes(Core.fromHex(hex));
const int = (n: bigint | number) => Core.PlutusData.newInteger(BigInt(n));
const constr = (index: number, fields: Core.PlutusData[] = []) => {
  const list = new Core.PlutusList();
  for (const f of fields) list.add(f);
  return Core.PlutusData.newConstrPlutusData(
    new Core.ConstrPlutusData(BigInt(index), list),
  );
};

/** UTF-8 text → hex, the on-chain representation of a token name. */
export const textToHex = (text: string) =>
  Core.toHex(new TextEncoder().encode(text));

// Redeemers of the gift_card policy's `Action` enum: Mint = Constr 0 [],
// Burn = Constr 1 []. The redeem validator ignores its redeemer (burning the
// token is the authorization), so it reuses the empty Constr 0.
const MINT = () => constr(0);
const BURN = () => constr(1);
const UNIT_REDEEMER = () => constr(0);

/**
 * Read back what `createGiftCard` stored in the inline datum:
 * Constr 0 [seed tx hash, seed output index, token name].
 */
function parseGiftCardDatum(utxo: Core.TransactionUnspentOutput): {
  seed: SeedUtxo;
  tokenNameHex: string;
} {
  const invalid = new Error("Gift-card UTxO has no inline datum of the expected shape.");
  const fields = utxo.output().datum()?.asInlineData()?.asConstrPlutusData()?.getData();
  if (fields?.getLength() !== 3) throw invalid;
  const txHash = fields.get(0).asBoundedBytes();
  const index = fields.get(1).asInteger();
  const tokenName = fields.get(2).asBoundedBytes();
  if (!txHash || index === undefined || !tokenName) throw invalid;
  return {
    seed: { txHash: Core.toHex(txHash), outputIndex: Number(index) },
    tokenNameHex: Core.toHex(tokenName),
  };
}

/** Anything Blaze can build with: any provider, any wallet. */
export type AnyBlaze = Blaze<Provider, Wallet>;

/**
 * Blockfrost answers 404 for an address that has never been used. Treat that
 * as "no UTxOs yet", so a fresh wallet or script address can be polled.
 */
async function emptyIfNotFound(
  query: Promise<Core.TransactionUnspentOutput[]>,
): Promise<Core.TransactionUnspentOutput[]> {
  try {
    return await query;
  } catch (err) {
    if ((err as { status?: number }).status === 404) return [];
    throw err;
  }
}

/**
 * Minimal core: derive the parameterised scripts, policy id, and redeem address
 * for a gift card, and build the two transactions. Pure derivation
 * (`getScripts`) touches no wallet or network, so it is safe to call from a
 * frontend to preview an address.
 */
export class GiftCardContract {
  private readonly blaze: AnyBlaze;
  private readonly giftCardCompiledCode: string;
  private readonly redeemCompiledCode: string;

  constructor(input: {
    /**
     * A Blaze instance (provider + wallet). Any wallet can build transactions —
     * a `ColdWallet` (read-only, e.g. a backend building for the user's
     * address) or a `WebWallet` (a CIP-30 browser wallet); `signAndSubmit`
     * needs one that can sign (`HotWallet`, `WebWallet`).
     */
    blaze: AnyBlaze;
    /** Override the bundled blueprint (defaults to the build-time bundle). */
    validators?: GiftCardValidators;
  }) {
    this.blaze = input.blaze;
    const validators = input.validators ?? getBundledValidators();
    // The blueprint's compiledCode is single-CBOR, which is what `applyParams`
    // expects; `cborToScript` wraps the result into a PlutusV3 script.
    this.giftCardCompiledCode = validators.giftCard;
    this.redeemCompiledCode = validators.redeem;
  }

  private get networkId(): Core.NetworkId {
    return this.blaze.provider.network;
  }

  /** Apply the (token name, seed UTxO) parameters to the one-shot mint policy. */
  private giftCardScriptFor(tokenNameHex: string, seed: SeedUtxo): Core.Script {
    // gift_card params: (token_name: ByteArray, utxo_ref: OutputReference).
    // OutputReference is Constr 0 [transaction_id (bare bytes), output_index].
    const applied = applyParams(
      Core.HexBlob(this.giftCardCompiledCode),
      bytes(tokenNameHex),
      constr(0, [bytes(seed.txHash), int(seed.outputIndex)]),
    );
    return cborToScript(applied, LANGUAGE_VERSION);
  }

  /** Apply the (token name, policy id) parameters to the redeem spend script. */
  private redeemScriptFor(tokenNameHex: string, policyId: string): Core.Script {
    const applied = applyParams(
      Core.HexBlob(this.redeemCompiledCode),
      bytes(tokenNameHex),
      bytes(policyId),
    );
    return cborToScript(applied, LANGUAGE_VERSION);
  }

  /** Derive every script and identifier from the hex token name + seed. */
  private scriptsFor(tokenNameHex: string, seed: SeedUtxo): GiftCardScripts {
    const giftCardScript = this.giftCardScriptFor(tokenNameHex, seed);
    const policyId = giftCardScript.hash();
    const redeemScript = this.redeemScriptFor(tokenNameHex, policyId);
    return {
      giftCardScript,
      policyId,
      redeemScript,
      redeemAddress: Core.addressFromValidator(this.networkId, redeemScript).toBech32(),
      unit: policyId + tokenNameHex,
    };
  }

  /**
   * Derive the parameterised scripts, policy id, and redeem address for a gift
   * card seeded by `seedUtxo`. Pure: no wallet or network access.
   */
  getScripts(tokenName: string, seedUtxo: SeedUtxo): GiftCardScripts {
    return this.scriptsFor(textToHex(tokenName), seedUtxo);
  }

  /**
   * Build a transaction that mints a unique gift-card token and locks
   * `giftLovelace` at the redeem script address. Returns the UNSIGNED tx —
   * sign and submit it with `signAndSubmit`, a browser wallet, or your own
   * signer.
   *
   * Also returns the (unique) redeem script address the gift lands at and its
   * asset unit — pass the address to `getGiftCardUtxoAt` to redeem.
   * Each gift card has its own address (the redeem script is parameterised by
   * the one-shot policy id), so the address alone identifies this gift card.
   */
  createGiftCard = async (
    tokenName: string,
    giftLovelace: bigint,
  ): Promise<GiftCardCreation> => {
    const utxos = await emptyIfNotFound(this.blaze.wallet.getUnspentOutputs());
    // Seed with a pure-ADA UTxO so the one-shot input doesn't drag tokens along.
    const seed =
      utxos.find((u) => (u.output().amount().multiasset()?.size ?? 0) === 0) ??
      utxos[0];
    if (seed === undefined) throw new Error("No UTxOs found in wallet");

    const seedRef: SeedUtxo = {
      txHash: seed.input().transactionId(),
      outputIndex: Number(seed.input().index()),
    };
    const tokenNameHex = textToHex(tokenName);
    const { giftCardScript, policyId, redeemAddress, unit } = this.scriptsFor(
      tokenNameHex,
      seedRef,
    );

    // Remember the seed reference + token name in the inline datum so `redeem`
    // can rebuild the parameterised scripts from just the on-chain UTxO. The
    // validator itself ignores the datum — burning the token is the auth.
    const datum = constr(0, [
      bytes(seedRef.txHash),
      int(seedRef.outputIndex),
      bytes(tokenNameHex),
    ]);

    const tx = await this.blaze
      .newTransaction()
      .addInput(seed)
      .addMint(
        Core.PolicyId(policyId),
        new Map([[Core.AssetName(tokenNameHex), 1n]]),
        MINT(),
      )
      .provideScript(giftCardScript)
      .lockAssets(
        Core.addressFromBech32(redeemAddress),
        new Core.Value(giftLovelace, new Map([[Core.AssetId(unit), 1n]])),
        datum,
      )
      .complete();

    return { tx, txCbor: tx.toCbor(), redeemAddress, unit, seedUtxo: seedRef };
  };

  /**
   * Find the gift-card UTxO locked at a redeem script address: the output
   * carrying an inline datum. `redeemAddress` is the bech32 address returned by
   * `createGiftCard`. Queried by address (not by tx hash) so it relies only on
   * the address-UTxO endpoint every provider supports.
   */
  getGiftCardUtxoAt = async (
    redeemAddress: string,
  ): Promise<Core.TransactionUnspentOutput | undefined> => {
    const utxos = await emptyIfNotFound(
      this.blaze.provider.getUnspentOutputs(Core.addressFromBech32(redeemAddress)),
    );
    return utxos.find((u) => u.output().datum()?.asInlineData() !== undefined);
  };

  /**
   * Build a transaction that burns a gift-card token and releases the locked
   * assets. `giftCardUtxo` sits at the redeem script address (see
   * `getGiftCardUtxoAt`). Returns the UNSIGNED tx.
   */
  redeemGiftCard = async (
    giftCardUtxo: Core.TransactionUnspentOutput,
  ): Promise<Core.Transaction> => {
    const { seed, tokenNameHex } = parseGiftCardDatum(giftCardUtxo);
    const { giftCardScript, redeemScript, policyId } = this.scriptsFor(tokenNameHex, seed);

    return this.blaze
      .newTransaction()
      .addInput(giftCardUtxo, UNIT_REDEEMER())
      .provideScript(redeemScript)
      .addMint(
        Core.PolicyId(policyId),
        new Map([[Core.AssetName(tokenNameHex), -1n]]),
        BURN(),
      )
      .provideScript(giftCardScript)
      .complete();
  };

  /**
   * Sign an unsigned transaction (a `Transaction` or its CBOR hex) with the
   * wallet and submit it. Needs a wallet that can sign. Returns the tx hash.
   */
  signAndSubmit = async (unsignedTx: Core.Transaction | string): Promise<string> => {
    const tx =
      typeof unsignedTx === "string"
        ? Core.Transaction.fromCbor(Core.TxCBOR(unsignedTx))
        : unsignedTx;
    const signed = await this.blaze.signTransaction(tx);
    return this.blaze.submitTransaction(signed);
  };
}

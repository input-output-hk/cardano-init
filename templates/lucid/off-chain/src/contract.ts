import {
  applyDoubleCborEncoding,
  applyParamsToScript,
  Constr,
  Data,
  fromText,
  mintingPolicyToId,
  validatorToAddress,
  type LucidEvolution,
  type MintingPolicy,
  type SpendingValidator,
  type UTxO,
} from "@lucid-evolution/lucid";

import { bundledBlueprint } from "./blueprint.generated.js";

// Framework-agnostic bindings for the GiftCard contract, built on Lucid
// Evolution. No Node-only dependencies (no `fs`), so this module is safe to
// import in a browser frontend as well as a backend: the on-chain blueprint is
// bundled in at build time from ../blueprint/plutus.json (see
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
  /** The unsigned create transaction as CBOR hex — sign and submit it. */
  txCbor: string;
  /** Bech32 redeem script address the gift lands at — pass to `getGiftCardUtxoAt`. */
  redeemAddress: string;
  /** The gift-card token's asset unit (policy id + hex token name). */
  unit: string;
};

/**
 * The parameterised scripts and derived identifiers for a single gift card.
 * Useful on a frontend to display the policy id / script address without
 * building a transaction.
 */
export type GiftCardScripts = {
  giftCardScript: MintingPolicy;
  policyId: string;
  redeemScript: SpendingValidator;
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

// Redeemers of the gift_card policy's `Action` enum: Mint = Constr 0 [],
// Burn = Constr 1 []. The redeem validator ignores its redeemer (burning the
// token is the authorization), so it reuses the empty Constr 0.
const MINT = Data.to(new Constr(0, []));
const BURN = Data.to(new Constr(1, []));
const UNIT_REDEEMER = Data.to(new Constr(0, []));

/**
 * Minimal core: derive the parameterised scripts, policy id, and redeem address
 * for a gift card, and build the two transactions. Pure derivation
 * (`getScripts`) touches no wallet or network, so it is safe to call from a
 * frontend to preview an address.
 */
export class GiftCardContract {
  private readonly lucid: LucidEvolution;
  private readonly giftCardCompiledCode: string;
  private readonly redeemCompiledCode: string;

  constructor(input: {
    /**
     * A Lucid instance with a wallet selected. Any wallet can build
     * transactions — `selectWallet.fromAddress` (read-only, e.g. a backend
     * building for the user's address) or `.fromAPI` (a CIP-30 browser wallet);
     * `signAndSubmit` needs one that can sign (`.fromSeed`, `.fromAPI`).
     */
    lucid: LucidEvolution;
    /** Override the bundled blueprint (defaults to the build-time bundle). */
    validators?: GiftCardValidators;
  }) {
    this.lucid = input.lucid;
    const validators = input.validators ?? getBundledValidators();
    // The blueprint's compiledCode is single-CBOR; Lucid scripts are double.
    this.giftCardCompiledCode = applyDoubleCborEncoding(validators.giftCard);
    this.redeemCompiledCode = applyDoubleCborEncoding(validators.redeem);
  }

  /** Apply the (token name, seed UTxO) parameters to the one-shot mint policy. */
  private giftCardScriptFor(
    tokenNameHex: string,
    seedTxHash: string,
    seedIndex: number,
  ): MintingPolicy {
    // gift_card params: (token_name: ByteArray, utxo_ref: OutputReference).
    // OutputReference is Constr 0 [transaction_id (bare bytes), output_index].
    return {
      type: LANGUAGE_VERSION,
      script: applyParamsToScript(this.giftCardCompiledCode, [
        tokenNameHex,
        new Constr(0, [seedTxHash, BigInt(seedIndex)]),
      ]),
    };
  }

  /** Apply the (token name, policy id) parameters to the redeem spend script. */
  private redeemScriptFor(
    tokenNameHex: string,
    policyId: string,
  ): SpendingValidator {
    return {
      type: LANGUAGE_VERSION,
      script: applyParamsToScript(this.redeemCompiledCode, [tokenNameHex, policyId]),
    };
  }

  /** Derive every script and identifier from the hex token name + seed. */
  private scriptsFor(tokenNameHex: string, seedUtxo: SeedUtxo): GiftCardScripts {
    const giftCardScript = this.giftCardScriptFor(
      tokenNameHex,
      seedUtxo.txHash,
      seedUtxo.outputIndex,
    );
    const policyId = mintingPolicyToId(giftCardScript);
    const redeemScript = this.redeemScriptFor(tokenNameHex, policyId);
    return {
      giftCardScript,
      policyId,
      redeemScript,
      redeemAddress: validatorToAddress(this.lucid.config().network!, redeemScript),
      unit: policyId + tokenNameHex,
    };
  }

  /**
   * Derive the parameterised scripts, policy id, and redeem address for a gift
   * card seeded by `seedUtxo`. Pure: no wallet or network access.
   */
  getScripts(tokenName: string, seedUtxo: SeedUtxo): GiftCardScripts {
    return this.scriptsFor(fromText(tokenName), seedUtxo);
  }

  /**
   * Build a transaction that mints a unique gift-card token and locks
   * `giftLovelace` at the redeem script address. Returns the UNSIGNED tx as
   * CBOR hex — sign and submit it with `signAndSubmit`, a browser wallet, or
   * your own signer.
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
    const utxos = await this.lucid.wallet().getUtxos();
    const seed = utxos[0];
    if (seed === undefined) throw new Error("No UTxOs found in wallet");

    const tokenNameHex = fromText(tokenName);
    const { giftCardScript, redeemAddress, unit } = this.scriptsFor(tokenNameHex, seed);

    // Remember the seed reference + token name in the inline datum so `redeem`
    // can rebuild the parameterised scripts from just the on-chain UTxO. The
    // validator itself ignores the datum — burning the token is the auth.
    const datum = Data.to(
      new Constr(0, [seed.txHash, BigInt(seed.outputIndex), tokenNameHex]),
    );

    const tx = await this.lucid
      .newTx()
      .collectFrom([seed])
      .mintAssets({ [unit]: 1n }, MINT)
      .attach.MintingPolicy(giftCardScript)
      .pay.ToContract(
        redeemAddress,
        { kind: "inline", value: datum },
        { lovelace: giftLovelace, [unit]: 1n },
      )
      .complete();

    return { txCbor: tx.toCBOR(), redeemAddress, unit };
  };

  /**
   * Find the gift-card UTxO locked at a redeem script address: the output
   * carrying an inline datum. `redeemAddress` is the bech32 address returned by
   * `createGiftCard`. Queried by address (not by tx hash) so it relies only on
   * the address-UTxO endpoint every provider supports.
   */
  getGiftCardUtxoAt = async (redeemAddress: string): Promise<UTxO | undefined> => {
    const utxos = await this.lucid.utxosAt(redeemAddress);
    return utxos.find((u) => typeof u.datum === "string" && u.datum !== "");
  };

  /**
   * Build a transaction that burns a gift-card token and releases the locked
   * assets. `giftCardUtxo` sits at the redeem script address (see
   * `getGiftCardUtxoAt`). Returns the UNSIGNED tx as CBOR hex.
   */
  redeemGiftCard = async (giftCardUtxo: UTxO): Promise<string> => {
    if (!giftCardUtxo.datum) throw new Error("Gift-card UTxO has no inline datum.");
    const data = Data.from(giftCardUtxo.datum);
    if (!(data instanceof Constr) || data.fields.length !== 3) {
      throw new Error("Unexpected datum shape.");
    }
    const [seedTxHash, seedIndex, tokenNameHex] = data.fields as [string, bigint, string];

    const { giftCardScript, redeemScript, unit } = this.scriptsFor(tokenNameHex, {
      txHash: seedTxHash,
      outputIndex: Number(seedIndex),
    });

    const tx = await this.lucid
      .newTx()
      .collectFrom([giftCardUtxo], UNIT_REDEEMER)
      .attach.SpendingValidator(redeemScript)
      .mintAssets({ [unit]: -1n }, BURN)
      .attach.MintingPolicy(giftCardScript)
      .complete();

    return tx.toCBOR();
  };

  /**
   * Sign an unsigned transaction with the selected wallet and submit it.
   * Needs a wallet that can sign. Returns the tx hash.
   */
  signAndSubmit = async (unsignedTx: string): Promise<string> => {
    const signed = await this.lucid.fromTx(unsignedTx).sign.withWallet().complete();
    return signed.submit();
  };
}

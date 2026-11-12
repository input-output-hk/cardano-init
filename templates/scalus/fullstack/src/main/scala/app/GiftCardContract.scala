package app

import scalus.cardano.blueprint.{
    Blueprint,
    Contract,
    HasTypeDescription,
    PlutusDataSchema,
    Preamble,
    TypeDescription,
    Validator
}
import scalus.cardano.ledger.MajorProtocolVersion
import scalus.cardano.onchain.plutus.v1.{PolicyId, TokenName}
import scalus.cardano.onchain.plutus.v3.TxOutRef
import scalus.compiler.Options
import scalus.uplc.builtin.{ByteString, Data}
import scalus.uplc.builtin.Data.toData
import scalus.uplc.{PlutusV3, Program}
import scalus.utils.Hex.toHex

/** Code for compiling, instantiating, and generating the CIP-57 blueprint for the two
  * gift-card validators.
  *
  * Extends [[Contract]] so the Scalus compiler plugin registers it and the `blueprint` sbt task
  * (from scalus-sbt-plugin) generates `META-INF/scalus/blueprints/GiftCardContract.json`.
  * `app.genBlueprint` (see Main.scala) writes the same CIP-57 JSON to the canonical
  * `../blueprint/plutus.json` so the other components can consume it.
  *
  * The blueprint exposes both validators under the conventional titles `giftcard.gift_card.mint`
  * and `giftcard.redeem.spend`, so off-chain tooling that locates validators by their
  * `<validator>.<purpose>` suffix (e.g. the MeshJS bindings) can find them.
  */
object GiftCardContract extends Contract {

    // Target Plomin (protocol 10) builtins, not the newest Scalus knows. Protocol 11 builtins
    // such as dropList break off-chain libraries that decode the script to apply parameters
    // (MeshJS's @harmoniclabs/uplc 1.4.1 rejects them), and the gift card doesn't need them.
    private given Options =
        Options.release.copy(targetProtocolVersion = MajorProtocolVersion.plominPV)

    /** Compiled gift-card minting policy: `\tokenName -> \seed -> \scriptContext -> Unit`. */
    val giftCardCompiled: PlutusV3[Data => Data => Data => Unit] =
        PlutusV3.compile(GiftCard.mint)

    /** Compiled redeem spending validator: `\tokenName -> \policyId -> \scriptContext -> Unit`. */
    val redeemCompiled: PlutusV3[Data => Data => Data => Unit] =
        PlutusV3.compile(Redeem.spend)

    /** Optimized UPLC program for the minting policy, with error traces for debugging. */
    val giftCardProgram: Program = giftCardCompiled.withErrorTraces.program

    /** Optimized UPLC program for the redeem validator, with error traces for debugging. */
    val redeemProgram: Program = redeemCompiled.withErrorTraces.program

    /** CIP-57 blueprint describing both gift-card validators. */
    lazy val blueprint: Blueprint = {
        val tokenNameParam = summon[HasTypeDescription[ByteString]].typeDescription
        // CIP-57 requires a constructor's `index`; Scalus leaves it out for single-constructor
        // types, so set it (TxOutRef is Constr 0).
        val seedParam = {
            val derived = summon[HasTypeDescription[TxOutRef]].typeDescription
            derived.copy(schema = derived.schema.copy(index = Some(0)))
        }
        val policyIdParam = summon[HasTypeDescription[ByteString]].typeDescription
        val actionSchema = summon[HasTypeDescription[Action]].typeDescription
        // The redeem validator ignores its redeemer, so any Plutus data is accepted. (Scalus's Unit
        // schema is the builtin `#unit`, which a redeemer can never be.)
        val anyDataSchema = TypeDescription(title = Some("Redeemer"), schema = PlutusDataSchema())
        Blueprint(
          preamble = Preamble(
            "Gift Card",
            "One-shot gift-card minting policy and its redeem spending validator",
            "1.0.0",
            plutusVersion = giftCardCompiled.language,
            license = Some("Apache-2.0")
          ),
          validators = Seq(
            Validator(
              title = "giftcard.gift_card.mint",
              description = Some(
                "One-shot minting policy: mints a unique gift-card token; burns it on redeem"
              ),
              redeemer = Some(actionSchema),
              datum = None,
              parameters = Some(List(tokenNameParam, seedParam)),
              compiledCode = Some(giftCardCompiled.program.cborEncoded.toHex),
              hash = Some(giftCardCompiled.script.scriptHash.toHex)
            ),
            Validator(
              title = "giftcard.redeem.spend",
              description = Some(
                "Guards the locked gift; releases the assets when the gift-card token is burned"
              ),
              redeemer = Some(anyDataSchema),
              datum = None,
              parameters = Some(List(tokenNameParam, policyIdParam)),
              compiledCode = Some(redeemCompiled.program.cborEncoded.toHex),
              hash = Some(redeemCompiled.script.scriptHash.toHex)
            )
          )
        )
    }

    /** Parameterise the one-shot minting policy with a token name and seed UTxO — applied as two
      * separate arguments, exactly as `applyParamsToScript` would. Every distinct seed yields a
      * fresh policy ID.
      */
    def makeGiftCardScript(tokenName: TokenName, seed: TxOutRef): PlutusV3[Data => Unit] = {
        val withTokenName = giftCardCompiled(tokenName.toData)
        withTokenName(seed.toData)
    }

    /** Parameterise the redeem spending validator with the token name and the gift-card policy ID,
      * applied as two separate arguments.
      */
    def makeRedeemScript(tokenName: TokenName, policyId: PolicyId): PlutusV3[Data => Unit] = {
        val withTokenName = redeemCompiled(tokenName.toData)
        withTokenName(policyId.toData)
    }
}

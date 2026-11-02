module GiftCard.MintingPolicy (
  PGiftCardAction (..),
  pgiftCardPolicy,
) where

import GHC.Generics (Generic)
import Generics.SOP qualified as SOP
import GiftCard.Utils (pcheck, pmintsExactly)
import Plutarch.LedgerApi.V3 (
  PScriptContext (..),
  PScriptInfo (PMintingScript),
  PTokenName,
  PTxInInfo (..),
  PTxInfo (..),
  PTxOutRef,
 )
import Plutarch.Prelude

{- | Redeemer for the gift-card minting policy: mint the one-shot token, or
burn it. 'DeriveAsDataStruct' encodes it as @Constr 0 []@ / @Constr 1 []@,
the same shape every off-chain template sends.
-}
data PGiftCardAction (s :: S) = PMint | PBurn
  deriving stock (Generic)
  deriving anyclass (SOP.Generic, PIsData)
  deriving (PlutusType, PValidateData) via (DeriveAsDataStruct PGiftCardAction)

{- | One-shot minting policy, parameterised by the gift-card token name and a
seed UTxO. Minting requires the seed UTxO to be spent (so the policy id is
unique) and mints exactly one token of the given name; burning requires
exactly one to be burned.

The two parameters are left free: off-chain code applies them to the compiled
script with @applyParamsToScript@, so every gift card gets a fresh policy id.
-}
pgiftCardPolicy ::
  forall (s :: S).
  Term s (PAsData PTokenName :--> PAsData PTxOutRef :--> PAsData PScriptContext :--> PUnit)
pgiftCardPolicy = plam $ \tokenName seed ctxData ->
  pmatch (pfromData ctxData) $ \ctx ->
    pmatch (pscriptContext'scriptInfo ctx) $ \case
      PMintingScript ownPolicy ->
        pmatch (pscriptContext'txInfo ctx) $ \txInfo ->
          let mintsOwn amount =
                pmintsExactly
                  # pfromData ownPolicy
                  # pfromData tokenName
                  # amount
                  # pfromData (ptxInfo'mint txInfo)
              seedSpent =
                pany
                  # plam (\input -> pmatch (pfromData input) $ \i -> pdata (ptxInInfo'outRef i) #== seed)
                  # pfromData (ptxInfo'inputs txInfo)
              -- The redeemer comes from the transaction builder, so check its shape.
              action = pfromData (pparseData @PGiftCardAction (pto (pscriptContext'redeemer ctx)))
           in pmatch action $ \case
                PMint -> pcheck "gift_card: seed not spent or not exactly one token minted" (seedSpent #&& mintsOwn 1)
                PBurn -> pcheck "gift_card: not exactly one token burned" (mintsOwn (-1))
      _ -> ptraceInfoError "gift_card: not a minting script"

module GiftCard.Redeem (predeemValidator) where

import GiftCard.Utils (pcheck, pmintsExactly)
import Plutarch.LedgerApi.V3 (
  PCurrencySymbol,
  PScriptContext (..),
  PTokenName,
  PTxInfo (..),
 )
import Plutarch.Prelude

{- | Spending validator guarding the gift locked at its address, parameterised
by the gift-card token name and the gift-card policy id. The UTxO can only be
spent when exactly that one token is burned in the same transaction. The datum
and redeemer are unused: burning the token is the authorisation.
-}
predeemValidator ::
  forall (s :: S).
  Term s (PAsData PTokenName :--> PAsData PCurrencySymbol :--> PAsData PScriptContext :--> PUnit)
predeemValidator = plam $ \tokenName policy ctxData ->
  pmatch (pfromData ctxData) $ \ctx ->
    pmatch (pscriptContext'txInfo ctx) $ \txInfo ->
      pcheck "redeem: gift-card token not burned" $
        pmintsExactly
          # pfromData policy
          # pfromData tokenName
          # (-1)
          # pfromData (ptxInfo'mint txInfo)

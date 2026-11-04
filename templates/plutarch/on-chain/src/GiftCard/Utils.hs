module GiftCard.Utils (pcheck, pmintsExactly) where

import Plutarch.LedgerApi.AssocMap qualified as AssocMap
import Plutarch.LedgerApi.V3 (PCurrencySymbol, PMintValue, PTokenName)
import Plutarch.Prelude

{- | True when the minted value holds exactly one entry under @policy@: the
token @tokenName@ with quantity @amount@ (@1@ to mint it, @-1@ to burn it).
Any other token of the same policy in the transaction makes it false.
-}
pmintsExactly ::
  forall (s :: S).
  Term s (PCurrencySymbol :--> PTokenName :--> PInteger :--> PMintValue :--> PBool)
pmintsExactly = phoistAcyclic $
  plam $ \policy tokenName amount mint ->
    pmatch (AssocMap.plookup # policy # pto (pto mint)) $ \case
      PNothing -> pconstant False
      PJust tokens -> tokens #== AssocMap.psingleton # tokenName # amount

-- | Succeed with unit when the condition holds, otherwise fail with a trace.
pcheck :: forall (s :: S). Term s PString -> Term s PBool -> Term s PUnit
pcheck message condition = pif condition (pconstant ()) (ptraceInfoError message)

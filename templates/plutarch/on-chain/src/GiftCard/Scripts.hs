-- | The validators compiled to Plutus Core, ready to serialise or evaluate.
module GiftCard.Scripts (giftCardScript, redeemScript) where

import Data.Text qualified as Text
import GiftCard.MintingPolicy (pgiftCardPolicy)
import GiftCard.Redeem (predeemValidator)
import Plutarch.Internal.Term (Config (NoTracing), compile)
import Plutarch.Prelude
import Plutarch.Script (Script)

-- | The gift-card minting policy, with its two parameters still unapplied.
giftCardScript :: Script
giftCardScript = compileOrFail "gift_card" pgiftCardPolicy

-- | The redeem spending validator, with its two parameters still unapplied.
redeemScript :: Script
redeemScript = compileOrFail "redeem" predeemValidator

{- | Compile without tracing, which keeps the scripts small. Switch to
@Tracing LogInfo DetTracing@ to keep the @ptraceInfo*@ messages while debugging.
-}
compileOrFail :: String -> (forall (s :: S). Term s a) -> Script
compileOrFail name term = case compile NoTracing term of
  Left err -> error ("failed to compile " <> name <> ": " <> Text.unpack err)
  Right script -> script

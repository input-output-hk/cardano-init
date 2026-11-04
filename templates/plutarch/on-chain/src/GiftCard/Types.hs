{-# LANGUAGE TemplateHaskell #-}

-- | Haskell-side mirrors of the on-chain types. They describe the wire format
-- in the CIP-57 blueprint and let tests build redeemers in that same format.
module GiftCard.Types (GiftCardAction (..)) where

import GHC.Generics (Generic)
import PlutusTx (makeIsDataSchemaIndexed)
import PlutusTx.Blueprint (HasBlueprintDefinition)

{- | Redeemer for the gift-card minting policy: mint the one-shot token, or burn
it. Encoded as @Constr 0 []@ / @Constr 1 []@, matching 'GiftCard.MintingPolicy.PGiftCardAction'.
-}
data GiftCardAction = Mint | Burn
  deriving stock (Generic)
  deriving anyclass (HasBlueprintDefinition)

makeIsDataSchemaIndexed ''GiftCardAction [('Mint, 0), ('Burn, 1)]

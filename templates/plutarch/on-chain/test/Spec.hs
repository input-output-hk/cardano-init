{- | Behaviour tests for the gift-card validators. Each case builds a script
context the way the ledger would, applies the parameters and context to the
compiled script exactly as an off-chain tool does, and evaluates it.
-}
module Main (main) where

import GiftCard.Scripts (giftCardScript, redeemScript)
import GiftCard.Types (GiftCardAction (Burn, Mint))
import Plutarch.Evaluate (applyArguments, evalScript)
import Plutarch.Script (Script)
import PlutusLedgerApi.V3
import PlutusLedgerApi.V3.MintValue (MintValue (UnsafeMintValue))
import PlutusTx.AssocMap qualified as Map
import PlutusTx.Builtins (dataToBuiltinData)
import Test.Tasty (TestTree, defaultMain, testGroup)
import Test.Tasty.HUnit (assertBool, testCase)

main :: IO ()
main =
  defaultMain $
    testGroup
      "gift card"
      [ testGroup
          "gift_card minting policy"
          [ accepts "mints one token when the seed is spent" $
              runGiftCard (redeemer Mint) [seed] (mintOf policy [(giftName, 1)])
          , rejects "minting without spending the seed" $
              runGiftCard (redeemer Mint) [otherRef] (mintOf policy [(giftName, 1)])
          , rejects "minting two tokens" $
              runGiftCard (redeemer Mint) [seed] (mintOf policy [(giftName, 2)])
          , rejects "minting a token with another name" $
              runGiftCard (redeemer Mint) [seed] (mintOf policy [("OTHER", 1)])
          , rejects "minting an extra token under the same policy" $
              runGiftCard (redeemer Mint) [seed] (mintOf policy [(giftName, 1), ("OTHER", 1)])
          , accepts "burns one token" $
              runGiftCard (redeemer Burn) [otherRef] (mintOf policy [(giftName, -1)])
          , rejects "burn redeemer while minting" $
              runGiftCard (redeemer Burn) [seed] (mintOf policy [(giftName, 1)])
          , rejects "a malformed redeemer" $
              runGiftCard (dataToBuiltinData (Constr 2 [])) [seed] (mintOf policy [(giftName, 1)])
          ]
      , testGroup
          "redeem spending validator"
          [ accepts "spends when the gift-card token is burned" $
              runRedeem (mintOf policy [(giftName, -1)])
          , rejects "spending without burning" $
              runRedeem (UnsafeMintValue Map.empty)
          , rejects "burning a token of another policy" $
              runRedeem (mintOf otherPolicy [(giftName, -1)])
          , rejects "minting instead of burning" $
              runRedeem (mintOf policy [(giftName, 1)])
          ]
      ]

accepts :: String -> Bool -> TestTree
accepts name result = testCase name (assertBool "expected the script to succeed" result)

rejects :: String -> Bool -> TestTree
rejects name result = testCase ("rejects " <> name) (assertBool "expected the script to fail" (not result))

-- | Apply the arguments to a compiled script and report whether it succeeds.
succeeds :: Script -> [Data] -> Bool
succeeds script args = case evalScript (applyArguments script args) of
  (Right _, _, _) -> True
  (Left _, _, _) -> False

-- | Run the minting policy with the given redeemer, spent inputs and mint value.
runGiftCard :: BuiltinData -> [TxOutRef] -> MintValue -> Bool
runGiftCard action spent mint =
  succeeds
    giftCardScript
    [ toData giftName
    , toData seed
    , toData (ScriptContext (txInfo spent mint) (Redeemer action) (MintingScript policy))
    ]

-- | Run the redeem validator against a transaction with the given mint value.
runRedeem :: MintValue -> Bool
runRedeem mint =
  succeeds
    redeemScript
    [ toData giftName
    , toData policy
    , toData (ScriptContext (txInfo [giftRef] mint) (Redeemer (toBuiltinData ())) (SpendingScript giftRef Nothing))
    ]

redeemer :: GiftCardAction -> BuiltinData
redeemer = toBuiltinData

giftName :: BuiltinByteString
giftName = "GIFT"

seed, otherRef, giftRef :: TxOutRef
seed = TxOutRef (TxId "seed-tx-0123456789abcdef01234567") 0
otherRef = TxOutRef (TxId "other-tx-0123456789abcdef0123456") 0
giftRef = TxOutRef (TxId "gift-tx-0123456789abcdef01234567") 0

policy, otherPolicy :: CurrencySymbol
policy = CurrencySymbol "gift-card-policy-0123456789a"
otherPolicy = CurrencySymbol "another-policy-0123456789abc"

-- | A mint value holding the given (token name, quantity) entries under one policy.
mintOf :: CurrencySymbol -> [(BuiltinByteString, Integer)] -> MintValue
mintOf cs tokens =
  UnsafeMintValue (Map.unsafeFromList [(cs, Map.unsafeFromList [(TokenName tn, q) | (tn, q) <- tokens])])

-- | A transaction spending the given outputs and minting the given value.
txInfo :: [TxOutRef] -> MintValue -> TxInfo
txInfo spent mint =
  TxInfo
    { txInfoInputs = map input spent
    , txInfoReferenceInputs = []
    , txInfoOutputs = []
    , txInfoFee = Lovelace 200_000
    , txInfoMint = mint
    , txInfoTxCerts = []
    , txInfoWdrl = Map.empty
    , txInfoValidRange = always
    , txInfoSignatories = []
    , txInfoRedeemers = Map.empty
    , txInfoData = Map.empty
    , txInfoId = TxId "this-tx-0123456789abcdef01234567"
    , txInfoVotes = Map.empty
    , txInfoProposalProcedures = []
    , txInfoCurrentTreasuryAmount = Nothing
    , txInfoTreasuryDonation = Nothing
    }
  where
    input ref =
      TxInInfo ref (TxOut (Address (PubKeyCredential (PubKeyHash "owner-key-hash-0123456789ab")) Nothing) mempty NoOutputDatum Nothing)

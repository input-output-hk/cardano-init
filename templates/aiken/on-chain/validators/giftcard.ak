use aiken/collection/dict
use aiken/collection/list
use cardano/assets.{PolicyId, from_asset}
use cardano/transaction.{Input, OutputReference, Transaction, placeholder}
use helpers.{mock_input, mock_output_reference, test_policy_id, test_token_name}

// The gift-card minting policy and the redeem spending validator.
// Creating a gift card mints a single token and locks the gift assets at the
// `redeem` script address. Redeeming burns that token and releases the assets
// to whoever spends the UTxO.

// --------------------------- MINT POLICY --------------------------------------
// ------------------------------------------------------------------------------

/// Redeemer for the gift-card minting policy.
pub type Action {
  Mint
  Burn
}

/// One-shot minting policy: parameterised by the token name and a specific
/// UTxO that must be spent when minting. Spending that UTxO can only happen
/// once, which guarantees the gift-card token is unique.
validator gift_card(token_name: ByteArray, utxo_ref: OutputReference) {
  mint(rdmr: Action, policy_id: PolicyId, tx: Transaction) {
    // Extract inputs and minted values from the transaction
    let Transaction { inputs, mint, .. } = tx
    // Get all tokens of this policy minted/burned in this transaction and
    // ensure it's only one.
    expect [Pair(asset_name, amount)] =
      mint
        |> assets.tokens(policy_id)
        |> dict.to_pairs()
    when rdmr is {
      Mint -> {
        // Check that we're spending the parameterised UTxO.
        expect Some(_input) =
          list.find(inputs, fn(input) { input.output_reference == utxo_ref })
        // Check that we're minting only one token of name `token_name`.
        amount == 1 && asset_name == token_name
      }
      // Check that we're burning only one token of name `token_name`.
      Burn -> amount == -1 && asset_name == token_name
    }
  }

  else(_) {
    fail
  }
}

// ------------------------ REDEEM (SPEND) VALIDATOR ----------------------------
// ------------------------------------------------------------------------------

/// Spending validator guarding the locked gift assets. It only allows the UTxO
/// to be spent when the matching gift-card token is being burned in the same
/// transaction.
validator redeem(token_name: ByteArray, policy_id: ByteArray) {
  spend(
    _datum: Option<Data>,
    _redeemer: Data,
    _input: OutputReference,
    tx: Transaction,
  ) {
    // Extract minted values from the transaction
    let Transaction { mint, .. } = tx
    // Get all tokens of this policy minted/burned in this transaction and
    // ensure it's only one.
    expect [Pair(asset_name, amount)] =
      mint
        |> assets.tokens(policy_id)
        |> dict.to_pairs()
    // Check that we're burning only one token of name `token_name`.
    amount == -1 && asset_name == token_name
  }

  else(_) {
    fail
  }
}

// --------------------------------- TESTS --------------------------------------
// ------------------------------------------------------------------------------

test success_mint() {
  let utxo_ref = mock_output_reference(0)
  let tx =
    Transaction {
      ..placeholder,
      inputs: [mock_input(utxo_ref)],
      mint: from_asset(test_policy_id, test_token_name, 1),
    }
  gift_card.mint(test_token_name, utxo_ref, Mint, test_policy_id, tx)
}

test fail_mint_with_wrong_amount() fail {
  let utxo_ref = mock_output_reference(0)
  let tx =
    Transaction {
      ..placeholder,
      inputs: [mock_input(utxo_ref)],
      mint: from_asset(test_policy_id, test_token_name, 2),
    }
  gift_card.mint(test_token_name, utxo_ref, Mint, test_policy_id, tx)
}

test fail_mint_without_param_utxo() fail {
  let utxo_ref = mock_output_reference(0)
  let tx =
    Transaction {
      ..placeholder,
      inputs: [mock_input(mock_output_reference(1))],
      mint: from_asset(test_policy_id, test_token_name, 1),
    }
  gift_card.mint(test_token_name, utxo_ref, Mint, test_policy_id, tx)
}

test success_burn_and_redeem() {
  let utxo_ref = mock_output_reference(0)
  let tx =
    Transaction {
      ..placeholder,
      mint: from_asset(test_policy_id, test_token_name, -1),
    }
  gift_card.mint(test_token_name, utxo_ref, Burn, test_policy_id, tx) && redeem.spend(
    test_token_name,
    test_policy_id,
    None,
    "",
    utxo_ref,
    tx,
  )
}

test fail_redeem_with_wrong_amount() fail {
  let utxo_ref = mock_output_reference(0)
  let tx =
    Transaction {
      ..placeholder,
      mint: from_asset(test_policy_id, test_token_name, -2),
    }
  redeem.spend(test_token_name, test_policy_id, None, "", utxo_ref, tx)
}

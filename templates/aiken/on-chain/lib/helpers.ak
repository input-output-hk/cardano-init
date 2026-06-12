use cardano/address
use cardano/assets.{zero}
use cardano/transaction.{Input, NoDatum, Output, OutputReference}

pub const test_token_name = "hello world"

pub const test_policy_id =
  #"00000000000000000000000000000000000000000000000000000000"

pub fn mock_output_reference(index: Int) -> OutputReference {
  OutputReference {
    transaction_id: #"0000000000000000000000000000000000000000000000000000000000000000",
    output_index: index,
  }
}

pub fn mock_input(output_reference: OutputReference) -> Input {
  Input {
    output_reference,
    output: Output {
      // A minimal payment address backed by a dummy verification-key hash.
      address: address.from_verification_key(
        #"00000000000000000000000000000000000000000000000000000000",
      ),
      value: zero,
      datum: NoDatum,
      reference_script: None,
    },
  }
}

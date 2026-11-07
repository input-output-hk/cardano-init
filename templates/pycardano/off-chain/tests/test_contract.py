from dataclasses import replace

import cbor2
import pytest
from pycardano import (
    AssetName,
    RawCBOR,
    TransactionId,
    TransactionInput,
    TransactionOutput,
    UTxO,
    Value,
)

from giftcard.contract import (
    GiftCardContract,
    GiftCardDatum,
    GiftCardValidators,
    OutputReference,
    build_with_evaluation_retry,
    find_validator,
    get_gift_card_validators,
    load_validators,
)

SEED = OutputReference(bytes(32), 0)
GIFT = AssetName(b"gift")


def test_retries_a_build_the_evaluator_rejected():
    outcomes = [ValueError("EvaluationFailure"), ValueError("EvaluationFailure"), "tx"]

    def build():
        outcome = outcomes.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        return outcome

    assert build_with_evaluation_retry(build, delay_seconds=0) == "tx"


def test_gives_up_retrying_after_the_last_attempt():
    calls = []

    def build():
        calls.append(True)
        raise ValueError("EvaluationFailure")

    with pytest.raises(ValueError, match="EvaluationFailure"):
        build_with_evaluation_retry(build, attempts=3, delay_seconds=0)
    assert len(calls) == 3


def test_does_not_retry_other_build_failures():
    calls = []

    def build():
        calls.append(True)
        raise ValueError("No UTxOs")

    with pytest.raises(ValueError, match="No UTxOs"):
        build_with_evaluation_retry(build, delay_seconds=0)
    assert len(calls) == 1


def test_finds_validators_by_suffix_whatever_the_module():
    blueprint = {
        "validators": [
            {"title": "any_module.gift_card.mint", "compiledCode": "aa"},
            {"title": "other.redeem.spend", "compiledCode": "bb"},
        ]
    }
    validators = get_gift_card_validators(blueprint)
    assert (validators.gift_card, validators.redeem) == ("aa", "bb")


def test_missing_validator_lists_what_is_available():
    with pytest.raises(ValueError, match="m.other.mint"):
        find_validator({"validators": [{"title": "m.other.mint", "compiledCode": ""}]}, "redeem.spend")


@pytest.mark.needs_blueprint
def test_derives_scripts_from_the_blueprint(context):
    scripts = GiftCardContract(context).get_scripts(b"hello world", SEED)
    assert len(scripts.policy_id.payload) == 28
    assert str(scripts.redeem_address).startswith("addr_test")
    assert scripts.redeem_address.payment_part.payload != scripts.policy_id.payload


@pytest.mark.needs_blueprint
def test_seed_changes_the_policy(context):
    contract = GiftCardContract(context)
    other_seed = OutputReference(bytes(32), 1)
    assert (
        contract.get_scripts(b"gift", SEED).policy_id
        != contract.get_scripts(b"gift", other_seed).policy_id
    )


@pytest.mark.needs_blueprint
def test_create_spends_the_seed_and_locks_the_token(context, wallet):
    creation = GiftCardContract(context).create_gift_card("gift", 5_000_000, wallet)
    body = creation.tx.transaction_body

    seed = TransactionInput(TransactionId(creation.seed.transaction_id), creation.seed.output_index)
    assert seed in body.inputs
    assert body.collateral
    assert body.script_data_hash is not None
    assert body.mint[creation.policy_id][GIFT] == 1

    locked = next(o for o in body.outputs if o.address == creation.redeem_address)
    assert locked.amount.coin == 5_000_000
    assert locked.amount.multi_asset[creation.policy_id][GIFT] == 1
    assert isinstance(locked.datum, GiftCardDatum)
    assert locked.datum.token_name == b"gift"
    assert creation.tx.transaction_witness_set.vkey_witnesses


@pytest.mark.needs_blueprint
def test_create_without_funds_fails(context, wallet):
    context.utxos_by_address.clear()
    with pytest.raises(ValueError, match="No UTxOs"):
        GiftCardContract(context).create_gift_card("gift", 5_000_000, wallet)


@pytest.mark.needs_blueprint
def test_redeem_burns_the_token_rebuilt_from_the_datum(context, wallet):
    contract = GiftCardContract(context)
    creation = contract.create_gift_card("gift", 5_000_000, wallet)
    body = creation.tx.transaction_body
    index, locked = next(
        (i, o) for i, o in enumerate(body.outputs) if o.address == creation.redeem_address
    )
    gift_card_utxo = UTxO(TransactionInput(TransactionId(bytes([9]) * 32), index), locked)
    context.utxos_by_address[str(creation.redeem_address)] = [gift_card_utxo]

    assert contract.find_gift_card_utxo(creation.redeem_address) == gift_card_utxo

    redeem_body = contract.redeem_gift_card(gift_card_utxo, wallet).transaction_body
    assert gift_card_utxo.input in redeem_body.inputs
    assert redeem_body.mint[creation.policy_id][GIFT] == -1
    assert not any(o.address == creation.redeem_address for o in redeem_body.outputs)


def _locked_gift_card(contract, wallet):
    creation = contract.create_gift_card("gift", 5_000_000, wallet)
    outputs = creation.tx.transaction_body.outputs
    locked = next(o for o in outputs if o.address == creation.redeem_address)
    return creation, locked


def _utxo(output, id_byte):
    return UTxO(TransactionInput(TransactionId(bytes([id_byte]) * 32), 0), output)


def _output_like(locked, datum):
    return TransactionOutput(locked.address, Value(2_000_000), datum=datum)


JUNK_DATUMS = {
    "not_cbor": lambda real: RawCBOR(b"\xff"),
    "wrong_shape": lambda real: RawCBOR(cbor2.dumps(42)),
    "right_datum_no_token": lambda real: RawCBOR(real.to_cbor()),
    "other_seed": lambda real: replace(real, seed_output_index=real.seed_output_index + 1),
    "wrong_field_types": lambda real: replace(real, token_name=7),
    "too_few_fields": lambda real: RawCBOR(cbor2.dumps(cbor2.CBORTag(121, [b"a"]))),
    "oversized_seed": lambda real: RawCBOR(
        cbor2.dumps(cbor2.CBORTag(121, [bytes(100), 0, real.token_name]))
    ),
    "oversized_token_name": lambda real: RawCBOR(
        cbor2.dumps(cbor2.CBORTag(121, [real.seed_tx_hash, 0, bytes(100)]))
    ),
}


@pytest.mark.needs_blueprint
@pytest.mark.parametrize("junk", JUNK_DATUMS.values(), ids=JUNK_DATUMS.keys())
def test_find_skips_whatever_else_was_sent_to_the_script(context, wallet, junk):
    contract = GiftCardContract(context)
    creation, locked = _locked_gift_card(contract, wallet)
    decoy = _utxo(_output_like(locked, junk(locked.datum)), 1)
    real = _utxo(locked, 2)

    context.utxos_by_address[str(creation.redeem_address)] = [decoy]
    assert contract.find_gift_card_utxo(creation.redeem_address) is None

    context.utxos_by_address[str(creation.redeem_address)] = [decoy, real]
    assert contract.find_gift_card_utxo(creation.redeem_address) == real


@pytest.mark.needs_blueprint
def test_find_reads_an_inline_datum_as_providers_return_it(context, wallet):
    contract = GiftCardContract(context)
    creation, locked = _locked_gift_card(contract, wallet)
    raw = TransactionOutput(locked.address, locked.amount, datum=RawCBOR(locked.datum.to_cbor()))
    utxo = _utxo(raw, 1)
    context.utxos_by_address[str(creation.redeem_address)] = [utxo]

    assert contract.find_gift_card_utxo(creation.redeem_address) == utxo
    body = contract.redeem_gift_card(utxo, wallet).transaction_body
    assert body.mint[creation.policy_id][GIFT] == -1


@pytest.mark.needs_blueprint
def test_redeem_rejects_a_utxo_that_is_not_a_gift_card(context, wallet):
    contract = GiftCardContract(context)
    _, locked = _locked_gift_card(contract, wallet)
    with pytest.raises(ValueError, match="not a gift card"):
        contract.redeem_gift_card(_utxo(_output_like(locked, RawCBOR(b"\xff")), 1), wallet)


@pytest.mark.needs_blueprint
def test_a_corrupt_blueprint_is_reported_not_mistaken_for_no_gift_card(context, wallet):
    creation, locked = _locked_gift_card(GiftCardContract(context), wallet)
    context.utxos_by_address[str(creation.redeem_address)] = [_utxo(locked, 1)]
    corrupt = GiftCardContract(context, GiftCardValidators(gift_card="4101", redeem="4101"))
    with pytest.raises(ValueError, match="Truncated"):
        corrupt.find_gift_card_utxo(creation.redeem_address)


@pytest.mark.needs_blueprint
def test_create_rejects_an_amount_below_the_minimum(context, wallet):
    with pytest.raises(ValueError, match="at least"):
        GiftCardContract(context).create_gift_card("gift", 1, wallet)


@pytest.mark.needs_blueprint
def test_create_accepts_exactly_the_minimum(context, wallet):
    contract = GiftCardContract(context)
    seed = contract.create_gift_card("gift", 5_000_000, wallet).seed
    minimum = contract.min_lock_lovelace(b"gift", seed)
    assert contract.create_gift_card("gift", minimum, wallet).tx
    with pytest.raises(ValueError, match=f"at least {minimum} "):
        contract.create_gift_card("gift", minimum - 1, wallet)


@pytest.mark.needs_blueprint
def test_blueprint_validators_are_present():
    validators = load_validators()
    assert validators.gift_card
    assert validators.redeem

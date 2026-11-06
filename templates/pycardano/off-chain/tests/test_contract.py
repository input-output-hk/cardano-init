import json
import threading
from dataclasses import replace
from fractions import Fraction
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

import cbor2
import pytest
from pycardano import (
    Address,
    AssetName,
    ChainContext,
    ExecutionUnits,
    GenesisParameters,
    Network,
    ProtocolParameters,
    RawCBOR,
    TransactionId,
    TransactionInput,
    TransactionOutput,
    UTxO,
    Value,
)

from giftcard.__main__ import create as create_command
from giftcard.contract import (
    GiftCardContract,
    GiftCardDatum,
    OutputReference,
    find_validator,
    get_gift_card_validators,
    has_blueprint,
    load_validators,
)
from giftcard.env import LocalBlockfrostContext, load_env, wallet_from_mnemonic

MNEMONIC = "test " * 23 + "sauce"
SEED = OutputReference(bytes(32), 0)
GIFT = AssetName(b"gift")

needs_blueprint = pytest.mark.skipif(
    not has_blueprint(), reason="build the on-chain component to run this"
)


class FakeContext(ChainContext):
    """An offline chain: fixed parameters and budgets, and UTxOs set by the test.

    It does not run the scripts; the devnet test in test_integration.py does.
    """

    def __init__(self):
        self.utxos_by_address = {}

    @property
    def protocol_param(self):
        return ProtocolParameters(
            min_fee_constant=155381,
            min_fee_coefficient=44,
            max_block_size=90112,
            max_tx_size=16384,
            max_block_header_size=1100,
            key_deposit=2000000,
            pool_deposit=500000000,
            pool_influence=Fraction(3, 10),
            monetary_expansion=Fraction(3, 1000),
            treasury_expansion=Fraction(1, 5),
            decentralization_param=Fraction(0),
            extra_entropy="",
            protocol_major_version=10,
            protocol_minor_version=0,
            min_utxo=0,
            min_pool_cost=170000000,
            price_mem=Fraction(577, 10000),
            price_step=Fraction(721, 10000000),
            max_tx_ex_mem=14000000,
            max_tx_ex_steps=10000000000,
            max_block_ex_mem=62000000,
            max_block_ex_steps=20000000000,
            max_val_size=5000,
            collateral_percent=150,
            max_collateral_inputs=3,
            coins_per_utxo_word=34482,
            coins_per_utxo_byte=4310,
            cost_models={"PlutusV3": {str(i): 0 for i in range(10)}},
            maximum_reference_scripts_size={"bytes": 200000},
            min_fee_reference_scripts={"base": 15, "range": 25600, "multiplier": 1.2},
        )

    @property
    def genesis_param(self):
        return GenesisParameters(
            active_slots_coefficient=Fraction(1, 20),
            update_quorum=5,
            max_lovelace_supply=45000000000000000,
            network_magic=2,
            epoch_length=86400,
            system_start=0,
            slots_per_kes_period=129600,
            slot_length=1,
            max_kes_evolutions=62,
            security_param=432,
        )

    @property
    def network(self):
        return Network.TESTNET

    @property
    def epoch(self):
        return 1

    @property
    def last_block_slot(self):
        return 1000

    def _utxos(self, address):
        return self.utxos_by_address.get(str(address), [])

    def evaluate_tx_cbor(self, cbor):
        budget = ExecutionUnits(100000, 50000000)
        return {"mint:0": budget, "spend:0": budget}


@pytest.fixture
def wallet():
    return wallet_from_mnemonic(MNEMONIC, Network.TESTNET)


@pytest.fixture
def context(wallet):
    fake = FakeContext()
    fake.utxos_by_address[str(wallet.address)] = [
        UTxO(TransactionInput(TransactionId(bytes([i]) * 32), 0), TransactionOutput(wallet.address, Value(coins)))
        for i, coins in enumerate([50_000_000, 1_000_000_000], start=1)
    ]
    return fake


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


def test_env_requires_a_mnemonic(monkeypatch):
    for key in ("MNEMONIC", "INDEXER_URL", "CARDANO_NETWORK", "BLOCKFROST_PROJECT_ID"):
        monkeypatch.delenv(key, raising=False)
    env, missing = load_env(files=[])
    assert env is None
    assert "MNEMONIC" in missing
    assert any(key.startswith("BLOCKFROST_PROJECT_ID") for key in missing)


def test_local_endpoint_needs_no_blockfrost_project(monkeypatch):
    monkeypatch.setenv("MNEMONIC", MNEMONIC)
    monkeypatch.setenv("INDEXER_URL", "http://localhost:8080/api/v1/")
    monkeypatch.delenv("BLOCKFROST_PROJECT_ID", raising=False)
    env, missing = load_env(files=[])
    assert missing == []
    assert env.indexer_url == "http://localhost:8080/api/v1"


@pytest.fixture
def yaci_like_server():
    """Serves /api/v1 only, and pages UTxOs with a default page size of 10."""
    total_utxos = 17

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            url = urlparse(self.path)
            query = parse_qs(url.query)
            if url.path == "/api/v1/epochs/latest":
                body = {"epoch": 1, "end_time": 4102444800}
            elif url.path.startswith("/api/v1/addresses/") and url.path.endswith("/utxos"):
                count = int(query.get("count", ["10"])[0])
                page = int(query.get("page", ["1"])[0])
                indexes = range(total_utxos)[(page - 1) * count : page * count]
                body = [
                    {
                        "tx_hash": "00" * 32,
                        "output_index": index,
                        "amount": [{"unit": "lovelace", "quantity": "2000000"}],
                        "data_hash": None,
                        "inline_datum": None,
                        "reference_script_hash": None,
                    }
                    for index in indexes
                ]
            else:
                self.send_error(404)
                return
            payload = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{server.server_port}/api/v1"
    server.shutdown()
    thread.join()


def test_local_context_uses_the_url_as_given_and_sees_every_utxo(yaci_like_server, wallet):
    context = LocalBlockfrostContext(yaci_like_server)
    assert len(context.utxos(wallet.address)) == 17


@needs_blueprint
def test_derives_scripts_from_the_blueprint(context):
    scripts = GiftCardContract(context).get_scripts(b"hello world", SEED)
    assert len(scripts.policy_id.payload) == 28
    assert str(scripts.redeem_address).startswith("addr_test")
    assert scripts.redeem_address.payment_part.payload != scripts.policy_id.payload


@needs_blueprint
def test_seed_changes_the_policy(context):
    contract = GiftCardContract(context)
    other_seed = OutputReference(bytes(32), 1)
    assert (
        contract.get_scripts(b"gift", SEED).policy_id
        != contract.get_scripts(b"gift", other_seed).policy_id
    )


@needs_blueprint
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


@needs_blueprint
def test_create_without_funds_fails(context, wallet):
    context.utxos_by_address.clear()
    with pytest.raises(ValueError, match="No UTxOs"):
        GiftCardContract(context).create_gift_card("gift", 5_000_000, wallet)


@needs_blueprint
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
}


@needs_blueprint
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


@needs_blueprint
def test_find_reads_an_inline_datum_as_providers_return_it(context, wallet):
    contract = GiftCardContract(context)
    creation, locked = _locked_gift_card(contract, wallet)
    raw = TransactionOutput(locked.address, locked.amount, datum=RawCBOR(locked.datum.to_cbor()))
    utxo = _utxo(raw, 1)
    context.utxos_by_address[str(creation.redeem_address)] = [utxo]

    assert contract.find_gift_card_utxo(creation.redeem_address) == utxo
    body = contract.redeem_gift_card(utxo, wallet).transaction_body
    assert body.mint[creation.policy_id][GIFT] == -1


@needs_blueprint
def test_redeem_rejects_a_utxo_that_is_not_a_gift_card(context, wallet):
    contract = GiftCardContract(context)
    _, locked = _locked_gift_card(contract, wallet)
    with pytest.raises(ValueError, match="not a gift card"):
        contract.redeem_gift_card(_utxo(_output_like(locked, RawCBOR(b"\xff")), 1), wallet)


@needs_blueprint
def test_blueprint_validators_are_present():
    validators = load_validators()
    assert validators.gift_card
    assert validators.redeem


@pytest.mark.parametrize(
    "token_name, lovelace, message",
    [
        ("gift", "abc", "whole number"),
        ("gift", "-1", "whole number"),
        ("gift", "0", "whole number"),
        ("gift", "1.5", "whole number"),
        ("x" * 33, "5000000", "32 bytes"),
    ],
)
def test_create_command_rejects_bad_input(token_name, lovelace, message):
    with pytest.raises(SystemExit, match=message):
        create_command(token_name, lovelace)


def test_wallet_address_matches_network():
    testnet = wallet_from_mnemonic(MNEMONIC, Network.TESTNET).address
    mainnet = wallet_from_mnemonic(MNEMONIC, Network.MAINNET).address
    assert isinstance(testnet, Address)
    assert str(testnet).startswith("addr_test")
    assert str(mainnet).startswith("addr1")

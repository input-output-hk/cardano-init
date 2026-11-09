from fractions import Fraction

import pytest
from pycardano import (
    ChainContext,
    ExecutionUnits,
    GenesisParameters,
    Network,
    ProtocolParameters,
    TransactionId,
    TransactionInput,
    TransactionOutput,
    UTxO,
    Value,
)

from giftcard.contract import has_blueprint
from giftcard.env import wallet_from_mnemonic

MNEMONIC = "test " * 23 + "sauce"


def pytest_runtest_setup(item):
    if item.get_closest_marker("needs_blueprint") and not has_blueprint():
        pytest.skip("build the on-chain component to run this")


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
def mnemonic():
    return MNEMONIC


@pytest.fixture
def wallet(mnemonic):
    return wallet_from_mnemonic(mnemonic, Network.TESTNET)


@pytest.fixture
def context(wallet):
    fake = FakeContext()
    fake.utxos_by_address[str(wallet.address)] = [
        UTxO(TransactionInput(TransactionId(bytes([i]) * 32), 0), TransactionOutput(wallet.address, Value(coins)))
        for i, coins in enumerate([50_000_000, 1_000_000_000], start=1)
    ]
    return fake

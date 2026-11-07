"""A full mint → lock → redeem round trip against a real local devnet (Yaci).

Runs only with a devnet (INDEXER_URL, written to ../.env by the devnet's `just
dev` or exported by its `just test`) and a built blueprint; otherwise it skips,
so `just test` stays green with no devnet. Select it with `pytest -m integration`.
"""

import os
import time

import pytest
from dotenv import dotenv_values
from pycardano import AssetName, Redeemer, TransactionBuilder, Unit

from giftcard.contract import (
    TTL_SLOTS,
    GiftCardContract,
    build_with_evaluation_retry,
    has_blueprint,
    is_evaluation_failure,
)
from giftcard.env import (
    DEFAULT_ENV_FILES,
    GiftCardEnv,
    create_context,
    topup_on_devnet,
    wallet_from_mnemonic,
)

pytestmark = pytest.mark.integration

# A throwaway devnet mnemonic, funded from the faucet. Never a real seed.
MNEMONIC = "test " * 23 + "sauce"
TIMEOUT_SECONDS = 120


def wait_for(find, timeout=TIMEOUT_SECONDS):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        found = find()
        if found:
            return found
        time.sleep(1)
    return find()


def _url(settings, key, default=""):
    return (settings.get(key) or default).strip().rstrip("/")


@pytest.fixture(scope="module")
def settings():
    """The dotenv files under the real environment, leaving `os.environ` untouched."""
    merged = {}
    for path in reversed(DEFAULT_ENV_FILES):
        merged.update(dotenv_values(path))
    merged.update(os.environ)
    return merged


# Under CARDANO_INIT_REQUIRE_DEVNET, a live devnet with no blueprint is a real
# breakage. The top-level `just test` also runs this suite with no devnet first;
# that skip is expected.
def test_blueprint_is_built_when_a_devnet_is_required(settings):
    if not _url(settings, "CARDANO_INIT_REQUIRE_DEVNET") or not _url(settings, "INDEXER_URL"):
        pytest.skip("no devnet is live")
    assert has_blueprint(), "Devnet is live but no blueprint: build the on-chain component first."


@pytest.fixture(scope="module")
def devnet(settings):
    indexer_url = _url(settings, "INDEXER_URL")
    if not indexer_url or not has_blueprint():
        pytest.skip("needs a devnet (INDEXER_URL) and a built blueprint")
    env = GiftCardEnv(
        network="preprod",
        mnemonic=MNEMONIC,
        indexer_url=indexer_url,
        yaci_admin_url=_url(settings, "YACI_ADMIN_URL", "http://localhost:10000"),
    )
    context = create_context(env)
    wallet = wallet_from_mnemonic(MNEMONIC, env.cardano_network)
    before = len(context.utxos(wallet.address))
    # A small UTxO usable as collateral and a large one to spend.
    topup_on_devnet(env, wallet.address, 10)
    topup_on_devnet(env, wallet.address, 10_000)
    assert wait_for(lambda: len(context.utxos(wallet.address)) >= before + 2)
    return GiftCardContract(context), wallet


def create(contract, wallet, token_name):
    creation = build_with_evaluation_retry(
        lambda: contract.create_gift_card(token_name, 5_000_000, wallet)
    )
    contract.context.submit_tx(creation.tx)
    utxo = wait_for(lambda: contract.find_gift_card_utxo(creation.redeem_address))
    assert utxo is not None, "the create tx should produce a gift-card UTxO"
    return creation, utxo


def test_mints_locks_and_redeems_a_gift_card(devnet):
    contract, wallet = devnet
    creation, utxo = create(contract, wallet, "IntegrationGift")
    assert utxo.output.amount.multi_asset[creation.policy_id][AssetName(b"IntegrationGift")] == 1

    tx = build_with_evaluation_retry(lambda: contract.redeem_gift_card(utxo, wallet))
    contract.context.submit_tx(tx)

    assert wait_for(lambda: contract.find_gift_card_utxo(creation.redeem_address) is None)
    assert not any(
        creation.policy_id in u.output.amount.multi_asset
        for u in contract.context.utxos(wallet.address)
    )


def test_spending_without_burning_is_rejected(devnet):
    contract, wallet = devnet
    creation, utxo = create(contract, wallet, "UnburnedGift")
    scripts = contract.get_scripts(b"UnburnedGift", creation.seed)

    def spend_without_burning():
        builder = TransactionBuilder(contract.context)
        builder.add_script_input(utxo, script=scripts.redeem_script, redeemer=Redeemer(Unit()))
        builder.add_input_address(wallet.address)
        builder.build(change_address=wallet.address, auto_ttl_offset=TTL_SLOTS)

    with pytest.raises(Exception) as rejected:
        spend_without_burning()
    # Not just any failure: the redeem script itself must have refused the spend.
    assert is_evaluation_failure(rejected.value), rejected.value
    assert "spend" in str(rejected.value) and "3012" in str(rejected.value), rejected.value

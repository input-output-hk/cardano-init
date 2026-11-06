"""CLI: `python -m giftcard [status | create <name> <lovelace> | redeem <address>]`."""

import logging
import sys
import time
from typing import Callable, TypeVar

from .contract import GiftCardContract, has_blueprint, is_evaluation_failure, load_validators
from .env import create_context, load_env, topup_on_devnet, wallet_from_mnemonic

T = TypeVar("T")

REDEEM_WAIT_SECONDS = 180
MAX_TOKEN_NAME_BYTES = 32
NO_BLUEPRINT = "No blueprint found. Build the on-chain component first: just -f ../on-chain/Justfile build"


def build_with_evaluation_retry(build: Callable[[], T]) -> T:
    """Retry while the provider's script evaluator catches up.

    A provider can report a tx as confirmed seconds before its evaluator knows the
    new outputs, so spending them right away briefly fails evaluation.
    """
    attempts = 6
    for attempt in range(1, attempts + 1):
        try:
            return build()
        except Exception as error:
            if attempt == attempts or not is_evaluation_failure(error):
                raise
            print("Script evaluation failed; the provider may still be indexing. Retrying in 10s…")
            time.sleep(10)
    raise AssertionError("unreachable")


def status() -> None:
    if not has_blueprint():
        print(NO_BLUEPRINT)
        return
    validators = load_validators()
    print("On-chain blueprint:")
    print(f"  gift_card (mint):  {len(validators.gift_card)} hex chars")
    print(f"  redeem    (spend): {len(validators.redeem)} hex chars")
    print()

    env, missing = load_env()
    if env is None:
        print("To submit transactions, set the following (see .env.example):")
        for key in missing:
            print(f"  - {key}")
    else:
        print("Environment ready. Run a transaction:")
        print("  just create <tokenName> <lovelace>")
        print("  just redeem <redeemAddress>")


def _require_env():
    if not has_blueprint():
        raise SystemExit(NO_BLUEPRINT)
    env, missing = load_env()
    if env is None:
        raise SystemExit(f"Missing required configuration: {', '.join(missing)}. See .env.example.")
    wallet = wallet_from_mnemonic(env.mnemonic, env.cardano_network)
    return env, GiftCardContract(create_context(env)), wallet


def create(token_name: str, lovelace: str) -> None:
    if len(token_name.encode()) > MAX_TOKEN_NAME_BYTES:
        raise SystemExit(f"Token name must be at most {MAX_TOKEN_NAME_BYTES} bytes.")
    if not lovelace.isdecimal() or int(lovelace) == 0:
        raise SystemExit(f'Lovelace must be a positive whole number, got "{lovelace}".')

    env, contract, wallet = _require_env()

    # A fresh devnet wallet needs funds and a separate UTxO usable as collateral.
    if topup_on_devnet(env, wallet.address, 10):
        topup_on_devnet(env, wallet.address, 10_000)
        print(f"Funded {str(wallet.address)[:20]}… from the devnet faucet.")
        for _ in range(60):
            if len(contract.context.utxos(wallet.address)) >= 2:
                break
            time.sleep(1)

    creation = build_with_evaluation_retry(
        lambda: contract.create_gift_card(token_name, int(lovelace), wallet)
    )
    contract.context.submit_tx(creation.tx)

    print(f'Gift card "{token_name}" created.')
    print(f"  tx:      {creation.tx.id}")
    print(f"  address: {creation.redeem_address}")
    print("Once it is confirmed on chain, redeem it with:")
    print(f"  just redeem {creation.redeem_address}")


def redeem(redeem_address: str) -> None:
    _, contract, wallet = _require_env()

    deadline = time.monotonic() + REDEEM_WAIT_SECONDS
    gift_card_utxo = contract.find_gift_card_utxo(redeem_address)
    while gift_card_utxo is None and time.monotonic() < deadline:
        time.sleep(3)
        gift_card_utxo = contract.find_gift_card_utxo(redeem_address)
    if gift_card_utxo is None:
        raise SystemExit(
            f"No gift-card UTxO found at {redeem_address} after {REDEEM_WAIT_SECONDS}s. "
            "Check that the create tx is confirmed, then retry."
        )

    tx = build_with_evaluation_retry(lambda: contract.redeem_gift_card(gift_card_utxo, wallet))
    contract.context.submit_tx(tx)

    print("Gift card redeemed; assets released to your wallet.")
    print(f"  tx: {tx.id}")


def main() -> None:
    # PyCardano logs its whole builder state whenever a build fails, including the
    # failures that `build_with_evaluation_retry` expects.
    logging.getLogger("PyCardano").setLevel(logging.CRITICAL)
    args = sys.argv[1:]
    command = args[0] if args else "status"
    if command == "status":
        status()
    elif command == "create" and len(args) == 3:
        create(args[1], args[2])
    elif command == "redeem" and len(args) == 2:
        redeem(args[1])
    else:
        raise SystemExit("usage: status | create <tokenName> <lovelace> | redeem <redeemAddress>")


if __name__ == "__main__":
    main()

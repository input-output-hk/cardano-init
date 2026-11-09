"""CLI: `giftcard [status | create <tokenName> <lovelace> | redeem <redeemAddress>]`."""

import argparse
import logging
import time
from collections.abc import Callable, Sequence
from typing import TypeVar

from .contract import (
    GiftCardContract,
    build_with_evaluation_retry,
    has_blueprint,
    is_collateral_shortage,
    load_validators,
)
from .env import ConfigError, create_context, load_env, topup_on_devnet, wallet_from_mnemonic

T = TypeVar("T")

REDEEM_WAIT_SECONDS = 180
MAX_TOKEN_NAME_BYTES = 32
NO_BLUEPRINT = "No blueprint found. Build the on-chain component first: just -f ../on-chain/Justfile build"


def build_tx(build: Callable[[], T]) -> T:
    """Build a tx, reporting problems the user can fix without a traceback."""
    try:
        return build_with_evaluation_retry(build)
    except ValueError as error:
        if is_collateral_shortage(error):
            raise SystemExit(
                f"Not enough collateral: {error}. Script transactions need wallet UTxOs "
                "above 2 ADA that add up to that amount, so send more ADA to the wallet."
            )
        raise SystemExit(str(error))


def status() -> None:
    if not has_blueprint():
        print(NO_BLUEPRINT)
        return
    validators = load_validators()
    print("On-chain blueprint:")
    print(f"  gift_card (mint):  {len(validators.gift_card)} hex chars")
    print(f"  redeem    (spend): {len(validators.redeem)} hex chars")
    print()

    try:
        load_env()
    except ConfigError as error:
        print("To submit transactions, set the following (see .env.example):")
        for key in error.missing:
            print(f"  - {key}")
        return
    print("Environment ready. Run a transaction:")
    print("  just create <tokenName> <lovelace>")
    print("  just redeem <redeemAddress>")


def _require_env():
    if not has_blueprint():
        raise SystemExit(NO_BLUEPRINT)
    try:
        env = load_env()
    except ConfigError as error:
        raise SystemExit(f"{error}. See .env.example.")
    wallet = wallet_from_mnemonic(env.mnemonic, env.cardano_network)
    return env, GiftCardContract(create_context(env)), wallet


def create(token_name: str, lovelace: int) -> None:
    env, contract, wallet = _require_env()

    # A fresh devnet wallet needs funds and a separate UTxO usable as collateral.
    if topup_on_devnet(env, wallet.address, 10):
        topup_on_devnet(env, wallet.address, 10_000)
        print(f"Funded {str(wallet.address)[:20]}… from the devnet faucet.")
        for _ in range(60):
            if len(contract.context.utxos(wallet.address)) >= 2:
                break
            time.sleep(1)

    creation = build_tx(lambda: contract.create_gift_card(token_name, lovelace, wallet))
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

    tx = build_tx(lambda: contract.redeem_gift_card(gift_card_utxo, wallet))
    contract.context.submit_tx(tx)

    print("Gift card redeemed; assets released to your wallet.")
    print(f"  tx: {tx.id}")


def _token_name(value: str) -> str:
    if len(value.encode()) > MAX_TOKEN_NAME_BYTES:
        raise argparse.ArgumentTypeError(f"must be at most {MAX_TOKEN_NAME_BYTES} bytes")
    return value


def _lovelace(value: str) -> int:
    try:
        amount = int(value)
    except ValueError:
        amount = 0
    if amount <= 0:
        raise argparse.ArgumentTypeError(f'must be a positive whole number, got "{value}"')
    return amount


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="giftcard", description="Create and redeem gift cards.")
    commands = parser.add_subparsers(dest="command")

    commands.add_parser("status", help="show the blueprint and whether the env is ready (default)")

    create_command = commands.add_parser("create", help="mint a token and lock lovelace with it")
    create_command.add_argument("token_name", type=_token_name, help="name of the gift card token")
    create_command.add_argument("lovelace", type=_lovelace, help="amount to lock")

    redeem_command = commands.add_parser("redeem", help="burn the token and release what it locks")
    redeem_command.add_argument("redeem_address", help="address printed by the create command")
    return parser


def main(argv: Sequence[str] | None = None) -> None:
    args = _parser().parse_args(argv)
    # PyCardano logs its whole builder state whenever a build fails, including the
    # failures that `build_with_evaluation_retry` expects.
    logging.getLogger("PyCardano").setLevel(logging.CRITICAL)
    if args.command == "create":
        create(args.token_name, args.lovelace)
    elif args.command == "redeem":
        redeem(args.redeem_address)
    else:
        status()


if __name__ == "__main__":
    main()

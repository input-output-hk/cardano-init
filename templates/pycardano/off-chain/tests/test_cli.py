import pytest
from pycardano import TransactionId, TransactionInput, TransactionOutput, UTxO, Value

import giftcard.__main__ as cli
from giftcard.contract import GiftCardContract, GiftCardValidators
from giftcard.env import ConfigError


@pytest.mark.needs_blueprint
def test_too_little_collateral_gets_a_clear_message(context, wallet):
    context.utxos_by_address[str(wallet.address)] = [
        UTxO(TransactionInput(TransactionId(bytes(32)), 0), TransactionOutput(wallet.address, Value(3_000_000)))
    ]
    contract = GiftCardContract(context)
    with pytest.raises(SystemExit, match="collateral"):
        cli.build_tx(lambda: contract.create_gift_card("gift", 1_500_000, wallet))


def test_other_build_errors_keep_their_own_message():
    def fail():
        raise ValueError("No UTxOs found in wallet")

    with pytest.raises(SystemExit, match="^No UTxOs found in wallet$"):
        cli.build_tx(fail)


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
def test_create_rejects_bad_input(capsys, token_name, lovelace, message):
    with pytest.raises(SystemExit):
        cli.main(["create", token_name, lovelace])
    assert message in capsys.readouterr().err


def test_create_converts_its_arguments(monkeypatch):
    calls = []
    monkeypatch.setattr(cli, "create", lambda *args: calls.append(args))
    cli.main(["create", "gift", "5000000"])
    assert calls == [("gift", 5_000_000)]


@pytest.mark.parametrize("argv", [[], ["status"]])
def test_status_is_the_default_command(monkeypatch, argv):
    calls = []
    monkeypatch.setattr(cli, "status", lambda: calls.append(True))
    cli.main(argv)
    assert calls == [True]


@pytest.fixture
def missing_config(monkeypatch):
    def load_env():
        raise ConfigError(["MNEMONIC"])

    monkeypatch.setattr(cli, "has_blueprint", lambda: True)
    monkeypatch.setattr(cli, "load_validators", lambda: GiftCardValidators("aa", "bb"))
    monkeypatch.setattr(cli, "load_env", load_env)


def test_status_lists_missing_config(missing_config, capsys):
    cli.status()
    assert "- MNEMONIC" in capsys.readouterr().out


def test_commands_exit_on_missing_config(missing_config):
    with pytest.raises(SystemExit, match="Missing configuration: MNEMONIC"):
        cli.redeem("addr_test1")

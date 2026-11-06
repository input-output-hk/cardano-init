"""Bindings for the GiftCard contract, built on PyCardano."""

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Optional, Tuple, Union

from pycardano import (
    Address,
    ChainContext,
    MultiAsset,
    PlutusData,
    PlutusV3Script,
    RawCBOR,
    RawPlutusData,
    Redeemer,
    ScriptHash,
    Transaction,
    TransactionBuilder,
    TransactionOutput,
    Unit,
    UTxO,
    Value,
    min_lovelace,
    plutus_script_hash,
)

from .env import Wallet
from .flat import apply_params

# src/giftcard/contract.py → the project root that holds blueprint/.
DEFAULT_BLUEPRINT_PATH = Path(__file__).resolve().parents[3] / "blueprint" / "plutus.json"

# PyCardano's default (10,000 slots past the tip) is beyond what a short-epoch
# devnet's node can convert to time, which fails script evaluation.
TTL_SLOTS = 300


def is_evaluation_failure(error: Exception) -> bool:
    """Whether a build failed because the provider's script evaluation did.

    Blockfrost reports that with a 200 (`TransactionFailedException` here), and
    Yaci Store with a 400 (`ApiError`), both carrying this Ogmios result.
    """
    return "EvaluationFailure" in str(error)


def is_collateral_shortage(error: Exception) -> bool:
    """Whether a build failed because the wallet can't cover a script tx's collateral."""
    return "Minimum collateral amount" in str(error)


@dataclass
class GiftCardValidators:
    """Compiled code (blueprint hex) of the two GiftCard validators."""

    gift_card: str
    redeem: str


def find_validator(blueprint: Dict[str, Any], suffix: str) -> str:
    """Find a validator by its `<validator>.<purpose>` title suffix.

    On-chain tools prefix titles with their module name, so the prefix is free.
    """
    for validator in blueprint["validators"]:
        title = validator["title"]
        if title == suffix or title.endswith(f".{suffix}"):
            return validator["compiledCode"]
    known = ", ".join(v["title"] for v in blueprint["validators"])
    raise ValueError(f'Validator "{suffix}" not found in blueprint. Available: {known}')


def get_gift_card_validators(blueprint: Dict[str, Any]) -> GiftCardValidators:
    return GiftCardValidators(
        gift_card=find_validator(blueprint, "gift_card.mint"),
        redeem=find_validator(blueprint, "redeem.spend"),
    )


def has_blueprint(path: Path = DEFAULT_BLUEPRINT_PATH) -> bool:
    return path.is_file()


def load_validators(path: Path = DEFAULT_BLUEPRINT_PATH) -> GiftCardValidators:
    if not path.is_file():
        raise FileNotFoundError(
            f"No blueprint at {path}. Build the on-chain component first "
            "(its `just build` writes blueprint/plutus.json)."
        )
    return get_gift_card_validators(json.loads(path.read_text()))


@dataclass
class OutputReference(PlutusData):
    """The seed `OutputReference`: Constr 0 [bare transaction id, output index]."""

    CONSTR_ID = 0
    transaction_id: bytes
    output_index: int


@dataclass
class Mint(PlutusData):
    CONSTR_ID = 0


@dataclass
class Burn(PlutusData):
    CONSTR_ID = 1


@dataclass
class GiftCardDatum(PlutusData):
    """Lets `redeem` rebuild the scripts from the UTxO alone. The validator ignores it."""

    CONSTR_ID = 0
    seed_tx_hash: bytes
    seed_output_index: int
    token_name: bytes


@dataclass
class GiftCardScripts:
    """The parameterised scripts and derived identifiers for one gift card."""

    gift_card_script: PlutusV3Script
    policy_id: ScriptHash
    redeem_script: PlutusV3Script
    redeem_address: Address


@dataclass
class GiftCardCreation:
    """The result of `create_gift_card`. `tx` is signed, ready to submit."""

    tx: Transaction
    redeem_address: Address
    policy_id: ScriptHash
    token_name: bytes
    seed: OutputReference


def _token(policy_id: ScriptHash, token_name: bytes, quantity: int) -> MultiAsset:
    return MultiAsset.from_primitive({policy_id.payload: {token_name: quantity}})


class GiftCardContract:
    def __init__(
        self,
        context: ChainContext,
        validators: Optional[GiftCardValidators] = None,
    ):
        self.context = context
        validators = validators or load_validators()
        self._gift_card_code = bytes.fromhex(validators.gift_card)
        self._redeem_code = bytes.fromhex(validators.redeem)

    def get_scripts(self, token_name: bytes, seed: OutputReference) -> GiftCardScripts:
        """Derive a gift card's scripts and identifiers. Touches no network."""
        name_param = RawPlutusData(token_name).to_cbor()
        gift_card_script = PlutusV3Script(
            apply_params(self._gift_card_code, [name_param, seed.to_cbor()])
        )
        policy_id = plutus_script_hash(gift_card_script)
        redeem_script = PlutusV3Script(
            apply_params(
                self._redeem_code,
                [name_param, RawPlutusData(policy_id.payload).to_cbor()],
            )
        )
        return GiftCardScripts(
            gift_card_script=gift_card_script,
            policy_id=policy_id,
            redeem_script=redeem_script,
            redeem_address=Address(
                plutus_script_hash(redeem_script), network=self.context.network
            ),
        )

    def create_gift_card(self, token_name: str, lovelace: int, wallet: Wallet) -> GiftCardCreation:
        """Build a signed tx that mints the token and locks `lovelace` with it."""
        utxos = self.context.utxos(wallet.address)
        if not utxos:
            raise ValueError("No UTxOs found in wallet")
        # A pure-ADA seed keeps the one-shot input from dragging tokens along.
        seed_utxo = next((u for u in utxos if not u.output.amount.multi_asset), utxos[0])
        seed = OutputReference(
            seed_utxo.input.transaction_id.payload, seed_utxo.input.index
        )
        name = token_name.encode()
        scripts = self.get_scripts(name, seed)
        token = _token(scripts.policy_id, name, 1)

        builder = TransactionBuilder(self.context)
        builder.add_input(seed_utxo)
        builder.add_input_address(wallet.address)
        builder.mint = token
        builder.add_minting_script(scripts.gift_card_script, Redeemer(Mint()))
        datum = GiftCardDatum(seed.transaction_id, seed.output_index, name)
        minimum = min_lovelace(
            self.context,
            output=TransactionOutput(scripts.redeem_address, Value(0, token), datum=datum),
        )
        if lovelace < minimum:
            raise ValueError(f"The gift card must lock at least {minimum} lovelace.")
        builder.add_output(
            TransactionOutput(scripts.redeem_address, Value(lovelace, token), datum=datum)
        )
        return GiftCardCreation(
            tx=builder.build_and_sign(
                [wallet.signing_key],
                change_address=wallet.address,
                merge_change=True,
                auto_ttl_offset=TTL_SLOTS,
            ),
            redeem_address=scripts.redeem_address,
            policy_id=scripts.policy_id,
            token_name=name,
            seed=seed,
        )

    def _read_gift_card(self, utxo: UTxO) -> Optional[Tuple[GiftCardDatum, GiftCardScripts]]:
        """The UTxO's datum and scripts, or None if it isn't a genuine gift card.

        Anyone can send anything to a script address, so a UTxO counts only if its
        datum derives this address and it holds the token that datum's policy mints.
        """
        datum = utxo.output.datum
        if datum is None:
            return None
        try:
            cbor = datum.cbor if isinstance(datum, RawCBOR) else datum.to_cbor()
            parsed = GiftCardDatum.from_cbor(cbor)
            seed = OutputReference(parsed.seed_tx_hash, parsed.seed_output_index)
        except Exception:
            return None
        scripts = self.get_scripts(parsed.token_name, seed)
        if utxo.output.address != scripts.redeem_address:
            return None
        if not _token(scripts.policy_id, parsed.token_name, 1) <= utxo.output.amount.multi_asset:
            return None
        return parsed, scripts

    def find_gift_card_utxo(self, redeem_address: Union[Address, str]) -> Optional[UTxO]:
        """Find the gift-card UTxO at a redeem address, ignoring anything else there."""
        utxos = self.context.utxos(redeem_address)
        return next((u for u in utxos if self._read_gift_card(u)), None)

    def redeem_gift_card(self, gift_card_utxo: UTxO, wallet: Wallet) -> Transaction:
        """Build a signed tx that burns the token and releases the locked assets."""
        gift_card = self._read_gift_card(gift_card_utxo)
        if gift_card is None:
            raise ValueError("This UTxO is not a gift card.")
        datum, scripts = gift_card

        builder = TransactionBuilder(self.context)
        # The redeem validator ignores its redeemer: burning the token is the auth.
        builder.add_script_input(
            gift_card_utxo, script=scripts.redeem_script, redeemer=Redeemer(Unit())
        )
        builder.add_input_address(wallet.address)
        builder.mint = _token(scripts.policy_id, datum.token_name, -1)
        builder.add_minting_script(scripts.gift_card_script, Redeemer(Burn()))
        return builder.build_and_sign(
            [wallet.signing_key],
            change_address=wallet.address,
            merge_change=True,
            auto_ttl_offset=TTL_SLOTS,
        )

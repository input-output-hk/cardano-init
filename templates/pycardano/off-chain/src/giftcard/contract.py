"""Bindings for the GiftCard contract, built on PyCardano."""

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Union

from pycardano import (
    Address,
    ChainContext,
    ExtendedSigningKey,
    MultiAsset,
    NonEmptyOrderedSet,
    PaymentSigningKey,
    PlutusData,
    PlutusV3Script,
    RawCBOR,
    Redeemer,
    ScriptHash,
    Transaction,
    TransactionBuilder,
    TransactionOutput,
    Unit,
    UTxO,
    Value,
    VerificationKeyWitness,
    plutus_script_hash,
)

from .flat import apply_params

# src/giftcard/contract.py → the project root that holds blueprint/.
DEFAULT_BLUEPRINT_PATH = Path(__file__).resolve().parents[3] / "blueprint" / "plutus.json"

# PyCardano's default (10,000 slots past the tip) is beyond what a short-epoch
# devnet's node can convert to time, which fails script evaluation.
TTL_SLOTS = 300

SigningKey = Union[PaymentSigningKey, ExtendedSigningKey]


def is_evaluation_failure(error: Exception) -> bool:
    """Whether a build failed because the provider's script evaluation did.

    Blockfrost reports that with a 200 (`TransactionFailedException` here), and
    Yaci Store with a 400 (`ApiError`), both carrying this Ogmios result.
    """
    return "EvaluationFailure" in str(error)


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


def _bytes_param(value: bytes) -> bytes:
    if len(value) > 64:
        raise ValueError("Byte-string parameters are limited to 64 bytes")
    prefix = bytes([0x40 | len(value)]) if len(value) < 24 else bytes([0x58, len(value)])
    return prefix + value


@dataclass
class GiftCardScripts:
    """The parameterised scripts and derived identifiers for one gift card."""

    gift_card_script: PlutusV3Script
    policy_id: ScriptHash
    redeem_script: PlutusV3Script
    redeem_address: Address
    token_name: bytes


@dataclass
class GiftCardCreation:
    """The result of `create_gift_card`. `tx` is unsigned."""

    tx: Transaction
    redeem_address: Address
    policy_id: ScriptHash
    token_name: bytes
    seed: OutputReference


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

    def _scripts_for(self, token_name: bytes, seed: OutputReference) -> GiftCardScripts:
        gift_card_script = PlutusV3Script(
            apply_params(self._gift_card_code, [_bytes_param(token_name), seed.to_cbor()])
        )
        policy_id = plutus_script_hash(gift_card_script)
        redeem_script = PlutusV3Script(
            apply_params(
                self._redeem_code,
                [_bytes_param(token_name), _bytes_param(policy_id.payload)],
            )
        )
        return GiftCardScripts(
            gift_card_script=gift_card_script,
            policy_id=policy_id,
            redeem_script=redeem_script,
            redeem_address=Address(
                plutus_script_hash(redeem_script), network=self.context.network
            ),
            token_name=token_name,
        )

    def get_scripts(self, token_name: str, seed: OutputReference) -> GiftCardScripts:
        """Derive a gift card's scripts and identifiers. Touches no network."""
        return self._scripts_for(token_name.encode(), seed)

    def _unsigned_tx(self, builder: TransactionBuilder, change_address: Address) -> Transaction:
        body = builder.build(
            change_address=change_address, merge_change=True, auto_ttl_offset=TTL_SLOTS
        )
        return Transaction(body, builder.build_witness_set(True))

    def create_gift_card(
        self, token_name: str, lovelace: int, wallet_address: Address
    ) -> GiftCardCreation:
        """Build an unsigned tx that mints the token and locks `lovelace` with it."""
        utxos = self.context.utxos(wallet_address)
        if not utxos:
            raise ValueError("No UTxOs found in wallet")
        # A pure-ADA seed keeps the one-shot input from dragging tokens along.
        seed_utxo = next((u for u in utxos if not u.output.amount.multi_asset), utxos[0])
        seed = OutputReference(
            seed_utxo.input.transaction_id.payload, seed_utxo.input.index
        )
        name = token_name.encode()
        scripts = self._scripts_for(name, seed)
        token = MultiAsset.from_primitive({scripts.policy_id.payload: {name: 1}})

        builder = TransactionBuilder(self.context)
        builder.add_input(seed_utxo)
        builder.add_input_address(wallet_address)
        builder.mint = token
        builder.add_minting_script(scripts.gift_card_script, Redeemer(Mint()))
        builder.add_output(
            TransactionOutput(
                scripts.redeem_address,
                Value(lovelace, token),
                datum=GiftCardDatum(seed.transaction_id, seed.output_index, name),
            )
        )
        return GiftCardCreation(
            tx=self._unsigned_tx(builder, wallet_address),
            redeem_address=scripts.redeem_address,
            policy_id=scripts.policy_id,
            token_name=name,
            seed=seed,
        )

    def find_gift_card_utxo(self, redeem_address: Union[Address, str]) -> Optional[UTxO]:
        """Find the gift-card UTxO at a redeem address: the one with an inline datum."""
        utxos = self.context.utxos(redeem_address)
        return next((u for u in utxos if u.output.datum is not None), None)

    def redeem_gift_card(self, gift_card_utxo: UTxO, wallet_address: Address) -> Transaction:
        """Build an unsigned tx that burns the token and releases the locked assets."""
        datum = gift_card_utxo.output.datum
        if datum is None:
            raise ValueError("Gift-card UTxO has no inline datum.")
        cbor = datum.cbor if isinstance(datum, RawCBOR) else datum.to_cbor()
        parsed = GiftCardDatum.from_cbor(cbor)
        seed = OutputReference(parsed.seed_tx_hash, parsed.seed_output_index)
        scripts = self._scripts_for(parsed.token_name, seed)

        builder = TransactionBuilder(self.context)
        # The redeem validator ignores its redeemer: burning the token is the auth.
        builder.add_script_input(
            gift_card_utxo, script=scripts.redeem_script, redeemer=Redeemer(Unit())
        )
        builder.add_input_address(wallet_address)
        builder.mint = MultiAsset.from_primitive(
            {scripts.policy_id.payload: {parsed.token_name: -1}}
        )
        builder.add_minting_script(scripts.gift_card_script, Redeemer(Burn()))
        return self._unsigned_tx(builder, wallet_address)

    def sign_and_submit(self, tx: Transaction, signing_keys: Sequence[SigningKey]) -> str:
        """Sign an unsigned transaction and submit it. Returns the tx hash."""
        tx_hash = tx.transaction_body.hash()
        witnesses: List[VerificationKeyWitness] = [
            VerificationKeyWitness(key.to_verification_key(), key.sign(tx_hash))
            for key in signing_keys
        ]
        tx.transaction_witness_set.vkey_witnesses = NonEmptyOrderedSet(witnesses)
        self.context.submit_tx(tx)
        return str(tx.id)


__all__ = [
    "Burn",
    "DEFAULT_BLUEPRINT_PATH",
    "GiftCardContract",
    "GiftCardCreation",
    "GiftCardDatum",
    "GiftCardScripts",
    "GiftCardValidators",
    "TTL_SLOTS",
    "Mint",
    "OutputReference",
    "find_validator",
    "get_gift_card_validators",
    "has_blueprint",
    "is_evaluation_failure",
    "load_validators",
]

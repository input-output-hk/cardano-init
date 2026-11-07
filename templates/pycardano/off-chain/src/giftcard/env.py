"""Configuration from the environment, and the provider + wallet it describes."""

import os
from collections.abc import Iterable
from dataclasses import dataclass
from functools import partial
from pathlib import Path

import requests
from blockfrost import ApiUrls, BlockFrostApi
from dotenv import load_dotenv
from pycardano import (
    Address,
    BlockFrostChainContext,
    ChainContext,
    ExtendedSigningKey,
    HDWallet,
    Network,
)

BLOCKFROST_URLS = {
    "preview": ApiUrls.preview.value,
    "preprod": ApiUrls.preprod.value,
    "mainnet": ApiUrls.mainnet.value,
}

_COMPONENT_DIR = Path(__file__).resolve().parents[2]
DEFAULT_ENV_FILES = (
    _COMPONENT_DIR.parent / ".env",
    _COMPONENT_DIR / ".env.local",
    _COMPONENT_DIR / ".env",
)


class ConfigError(Exception):
    def __init__(self, missing: list[str]):
        super().__init__(f"Missing configuration: {', '.join(missing)}")
        self.missing = missing


@dataclass(frozen=True)
class GiftCardEnv:
    """How to reach the chain, decided by the shared ../.env connection seam.

    `indexer_url` set means a local Blockfrost-compatible API; otherwise public
    Blockfrost, which needs `blockfrost_project_id`. `yaci_admin_url` set means
    that API is a Yaci devnet with a faucet.
    """

    network: str
    mnemonic: str
    indexer_url: str = ""
    yaci_admin_url: str = ""
    blockfrost_project_id: str = ""

    @property
    def cardano_network(self) -> Network:
        return Network.MAINNET if self.network == "mainnet" else Network.TESTNET


def load_env(files: Iterable[Path] = DEFAULT_ENV_FILES) -> GiftCardEnv:
    """Read the dotenv files without overriding what is already set.

    Raises `ConfigError`, listing what is missing, if the result is incomplete.
    """
    for path in files:
        load_dotenv(path, override=False)

    def read(key: str) -> str:
        return os.environ.get(key, "").strip()

    network = read("CARDANO_NETWORK") or "preview"
    indexer_url = read("INDEXER_URL").rstrip("/")
    project_id = read("BLOCKFROST_PROJECT_ID")
    mnemonic = read("MNEMONIC")

    missing = []
    if network not in BLOCKFROST_URLS:
        missing.append("CARDANO_NETWORK (one of preview|preprod|mainnet)")
    if not indexer_url and not project_id:
        missing.append("BLOCKFROST_PROJECT_ID (or start a local devnet: INDEXER_URL)")
    if not mnemonic:
        missing.append("MNEMONIC")
    if missing:
        raise ConfigError(missing)

    return GiftCardEnv(
        network=network,
        mnemonic=mnemonic,
        indexer_url=indexer_url,
        yaci_admin_url=read("YACI_ADMIN_URL").rstrip("/"),
        blockfrost_project_id=project_id,
    )


class LocalBlockfrostContext(BlockFrostChainContext):
    """A Blockfrost context for any Blockfrost-compatible base URL.

    The stock context always appends the `/v0` API version, but a local
    INDEXER_URL such as Yaci Store's `/api/v1` is already complete. It also lists
    UTxOs without a page size, and the client stops gathering pages at the first
    one shorter than 100, so a server whose default page is smaller (Yaci Store's
    is 10) would hide the rest.
    """

    def __init__(self, base_url: str, network: Network = Network.TESTNET):
        self._network = network
        self._project_id = "local"
        self._base_url = base_url
        self.api = BlockFrostApi(project_id=self._project_id, base_url=base_url)
        self.api.api_version = None
        self.api.address_utxos = partial(self.api.address_utxos, count=100)
        self._epoch_info = self.api.epoch_latest()
        self._epoch = None
        self._genesis_param = None
        self._protocol_param = None


def create_context(env: GiftCardEnv) -> ChainContext:
    if env.indexer_url:
        return LocalBlockfrostContext(env.indexer_url, env.cardano_network)
    return BlockFrostChainContext(
        env.blockfrost_project_id, base_url=BLOCKFROST_URLS[env.network]
    )


@dataclass(frozen=True)
class Wallet:
    signing_key: ExtendedSigningKey
    address: Address


def wallet_from_mnemonic(mnemonic: str, network: Network) -> Wallet:
    """Derive the first CIP-1852 account's base address and payment key."""
    root = HDWallet.from_mnemonic(mnemonic)
    payment = ExtendedSigningKey.from_hdwallet(root.derive_from_path("m/1852'/1815'/0'/0/0"))
    stake = ExtendedSigningKey.from_hdwallet(root.derive_from_path("m/1852'/1815'/0'/2/0"))
    address = Address(
        payment.to_verification_key().hash(),
        stake.to_verification_key().hash(),
        network=network,
    )
    return Wallet(payment, address)


def topup_on_devnet(env: GiftCardEnv, address: Address, ada: int) -> bool:
    """Fund an address from a Yaci devnet's faucet. `ada` is in ADA, not lovelace.

    Returns False, doing nothing, on any other provider.
    """
    if not env.yaci_admin_url:
        return False
    response = requests.post(
        f"{env.yaci_admin_url}/local-cluster/api/addresses/topup",
        json={"address": str(address), "adaAmount": ada},
        timeout=30,
    )
    response.raise_for_status()
    return True

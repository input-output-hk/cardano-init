import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlparse

import pytest
from pycardano import Address, Network

from giftcard.env import ConfigError, LocalBlockfrostContext, load_env, wallet_from_mnemonic


def test_env_requires_a_mnemonic(monkeypatch):
    for key in ("MNEMONIC", "INDEXER_URL", "CARDANO_NETWORK", "BLOCKFROST_PROJECT_ID"):
        monkeypatch.delenv(key, raising=False)
    with pytest.raises(ConfigError) as error:
        load_env(files=[])
    assert "MNEMONIC" in error.value.missing
    assert any(key.startswith("BLOCKFROST_PROJECT_ID") for key in error.value.missing)
    assert "MNEMONIC" in str(error.value)


def test_local_endpoint_needs_no_blockfrost_project(monkeypatch, mnemonic):
    monkeypatch.setenv("MNEMONIC", mnemonic)
    monkeypatch.setenv("INDEXER_URL", "http://localhost:8080/api/v1/")
    monkeypatch.delenv("BLOCKFROST_PROJECT_ID", raising=False)
    assert load_env(files=[]).indexer_url == "http://localhost:8080/api/v1"


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


def test_wallet_address_matches_network(mnemonic):
    testnet = wallet_from_mnemonic(mnemonic, Network.TESTNET).address
    mainnet = wallet_from_mnemonic(mnemonic, Network.MAINNET).address
    assert isinstance(testnet, Address)
    assert str(testnet).startswith("addr_test")
    assert str(mainnet).startswith("addr1")

// Assemble the CIP-57 blueprint from the flat UPLC files `pebble compile`
// writes. Pebble has no blueprint output of its own, so `just build` runs this
// after compiling each validator.
//
// Usage: node scripts/blueprint.mjs <out-path>
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { blake2b_224 } = require("@harmoniclabs/crypto");
const pebble = require("@harmoniclabs/pebble-cli/package.json");
const pkg = JSON.parse(readFileSync("package.json", "utf8"));

const out = process.argv[2];
if (!out) {
  console.error("usage: node scripts/blueprint.mjs <out-path>");
  process.exit(2);
}

// Wrap bytes in a CBOR byte string (major type 2).
function cborBytes(bytes) {
  const n = bytes.length;
  let head;
  if (n < 24) head = [0x40 | n];
  else if (n < 0x100) head = [0x58, n];
  else if (n < 0x10000) head = [0x59, n >> 8, n & 0xff];
  else head = [0x5a, (n >>> 24) & 0xff, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
  return Buffer.concat([Buffer.from(head), bytes]);
}

const hex = (bytes) => Buffer.from(bytes).toString("hex");

function validator(title, flatPath, redeemer, parameters) {
  // compiledCode is the flat program wrapped once in CBOR: the shape
  // `aiken build` emits and every off-chain template expects.
  const script = cborBytes(readFileSync(flatPath));
  // PlutusV3 script hash = blake2b-224(0x03 ‖ script).
  const hash = blake2b_224(new Uint8Array(Buffer.concat([Buffer.from([0x03]), script])));
  return { title, redeemer, parameters, compiledCode: hex(script), hash: hex(hash) };
}

const bytes = { dataType: "bytes" };
const outputReference = {
  title: "OutputReference",
  anyOf: [
    {
      title: "OutputReference",
      dataType: "constructor",
      index: 0,
      fields: [
        { title: "transaction_id", dataType: "bytes" },
        { title: "output_index", dataType: "integer" },
      ],
    },
  ],
};

// Titles follow the shared `<module>.<validator>.<purpose>` convention:
// off-chain code finds the validators by their `gift_card.mint` and
// `redeem.spend` suffixes.
const blueprint = {
  preamble: {
    title: pkg.name,
    description: "Gift-card validators compiled with Pebble",
    version: pkg.version,
    plutusVersion: "v3",
    compiler: { name: "pebble", version: pebble.version },
  },
  validators: [
    validator(
      "giftcard.gift_card.mint",
      "out/gift_card/out.flat",
      {
        title: "action",
        schema: {
          title: "Action",
          anyOf: [
            { title: "Mint", dataType: "constructor", index: 0, fields: [] },
            { title: "Burn", dataType: "constructor", index: 1, fields: [] },
          ],
        },
      },
      [
        { title: "token_name", schema: bytes },
        { title: "utxo_ref", schema: outputReference },
      ],
    ),
    validator(
      "giftcard.redeem.spend",
      "out/redeem/out.flat",
      { title: "_redeemer", schema: {} },
      [
        { title: "token_name", schema: bytes },
        { title: "policy_id", schema: bytes },
      ],
    ),
  ],
};

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, JSON.stringify(blueprint, null, 2) + "\n");
console.log(`Blueprint written to ${out}`);

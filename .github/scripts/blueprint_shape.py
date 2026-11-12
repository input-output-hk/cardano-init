#!/usr/bin/env python3
"""Check that an on-chain tool's blueprint matches the reference shape.

Usage: blueprint_shape.py BLUEPRINT [--write]

Every off-chain template reads the same things from blueprint/plutus.json: the
`gift_card.mint` and `redeem.spend` validators (found by title suffix), their
parameters, applied in order, the redeemer encoding, the Plutus version, and
`compiledCode` with its CBOR wrapping, UPLC version, and the builtins it uses
(off-chain libraries that decode the script to apply parameters reject builtins
newer than they know). This script reduces a
blueprint to exactly those things (its "shape") and compares it with
.github/blueprint-shape.json, which is Aiken's shape.

Matching shapes mean any off-chain tool that works with Aiken's blueprint reads
this one the same way. Script bytes and hashes are expected to differ, and so
are titles, descriptions, definition names, and whether a schema is inline or a
$ref. The spend validator's datum is not compared: no off-chain template reads
it from the blueprint, and the validators ignore it.

--write replaces the reference with BLUEPRINT's shape. Use it only with an
Aiken blueprint, after an intentional change to the contract.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REFERENCE = ROOT / ".github" / "blueprint-shape.json"

# The validators off-chain templates look up, by title suffix.
CONSUMED = ("gift_card.mint", "redeem.spend")

# Schema keys that only document; they don't change how data is encoded.
DOC_KEYS = {"title", "description", "$comment", "purpose"}


def resolve_ref(ref, definitions):
    prefix = "#/definitions/"
    if not ref.startswith(prefix):
        raise ValueError(f"unsupported $ref: {ref}")
    name = ref[len(prefix):].replace("~1", "/").replace("~0", "~")
    return definitions[name]


def normalize(schema, definitions, depth=0):
    """A schema reduced to its encoding: refs inlined, doc keys dropped."""
    if depth > 32:
        raise ValueError("schema nests too deep (recursive $ref?)")
    while isinstance(schema, dict) and "$ref" in schema:
        schema = resolve_ref(schema["$ref"], definitions)
    if isinstance(schema, list):
        return [normalize(s, definitions, depth + 1) for s in schema]
    if not isinstance(schema, dict):
        return schema
    out = {k: normalize(v, definitions, depth + 1) for k, v in schema.items() if k not in DOC_KEYS}
    # Constructors with distinct indices can't overlap, so `oneOf` (Plinth, Plutarch)
    # accepts exactly what `anyOf` (Aiken) does.
    alternatives = out.get("oneOf")
    if isinstance(alternatives, list) and "anyOf" not in out:
        indices = [a.get("index") if isinstance(a, dict) and a.get("dataType") == "constructor" else None
                   for a in alternatives]
        if None not in indices and len(set(indices)) == len(indices):
            out["anyOf"] = out.pop("oneOf")
    # A single-constructor type may be written with or without an anyOf wrapper.
    if set(out) == {"anyOf"} and len(out["anyOf"]) == 1:
        return out["anyOf"][0]
    # `{}` and a bare "Data" definition both mean "any Plutus data".
    return out or "any"


def cbor_bytestring(data):
    """The payload of `data` if it is exactly one CBOR byte string, else None."""
    if not data or data[0] >> 5 != 2:
        return None
    info = data[0] & 0x1F
    if info < 24:
        size, start = info, 1
    elif info in (24, 25, 26, 27):
        width = 1 << (info - 24)
        size, start = int.from_bytes(data[1:1 + width], "big"), 1 + width
    else:
        return None  # indefinite length: not used for scripts
    return data[start:] if len(data) - start == size else None


# The newest builtin in Plomin (protocol 10) is ripemd_160, number 86. Later ones
# (expModInteger, dropList, arrays, ...) arrived with protocol 11, and MeshJS's
# @harmoniclabs/uplc 1.4.1 fails to decode a script that uses them.
NEWEST_PLOMIN_BUILTIN = 86


class FlatReader:
    """Walks a Flat-encoded UPLC program (Plutus Core spec, appendix "Flat serialisation")."""

    def __init__(self, program):
        self.bits = "".join(f"{byte:08b}" for byte in program)
        self.pos = 0

    def take(self, count):
        if self.pos + count > len(self.bits):
            raise ValueError("truncated UPLC program")
        value = int(self.bits[self.pos:self.pos + count], 2)
        self.pos += count
        return value

    def natural(self):
        value, shift = 0, 0
        while True:
            chunk = self.take(8)
            value |= (chunk & 0x7F) << shift
            shift += 7
            if not chunk & 0x80:
                return value

    def skip_filler(self):
        while self.take(1) == 0:
            pass

    def skip_bytes(self):
        self.skip_filler()
        while (length := self.take(8)) != 0:
            self.pos += length * 8

    def read_type(self, tags):
        tag = tags.pop(0)
        if tag != 7:  # not a type application
            return (tag,)
        if tags[:2] == [7, 6]:  # pair
            del tags[:2]
            return (6, self.read_type(tags), self.read_type(tags))
        if tags[:1] == [5]:  # list
            tags.pop(0)
            return (5, self.read_type(tags))
        raise ValueError(f"unsupported constant type application: {tags}")

    def skip_value(self, kind):
        tag = kind[0]
        if tag == 0:  # integer
            self.natural()
        elif tag in (1, 2, 8):  # bytestring, string, data
            self.skip_bytes()
        elif tag == 4:  # bool
            self.pos += 1
        elif tag == 5:  # list
            while self.take(1):
                self.skip_value(kind[1])
        elif tag == 6:  # pair
            self.skip_value(kind[1])
            self.skip_value(kind[2])
        elif tag != 3:  # unit
            raise ValueError(f"unsupported constant type: {tag}")

    def builtins(self):
        """The set of builtin numbers the program's term uses."""
        for _ in range(3):  # version
            self.natural()
        found = set()
        # Iterative: compiled scripts nest deeper than Python's recursion limit.
        stack = ["term"]
        while stack:
            if stack.pop() == "list":
                if self.take(1):
                    stack += ["list", "term"]
                continue
            tag = self.take(4)
            if tag == 0:  # var
                self.natural()
            elif tag in (1, 2, 5):  # delay, lam, force
                stack.append("term")
            elif tag == 3:  # apply
                stack += ["term", "term"]
            elif tag == 4:  # constant
                tags = []
                while self.take(1):
                    tags.append(self.take(4))
                self.skip_value(self.read_type(tags))
            elif tag == 7:  # builtin
                found.add(self.take(7))
            elif tag == 8:  # constr
                self.natural()
                stack.append("list")
            elif tag == 9:  # case
                stack += ["list", "term"]
            elif tag != 6:  # error
                raise ValueError(f"unknown UPLC term tag: {tag}")
        return found


def code_shape(compiled_code):
    data = bytes.fromhex(compiled_code)
    layers = 0
    while (inner := cbor_bytestring(data)) is not None:
        data, layers = inner, layers + 1
    newer = sorted(b for b in FlatReader(data).builtins() if b > NEWEST_PLOMIN_BUILTIN)
    return {
        "cborLayers": layers,
        # Flat-encoded UPLC opens with the program version as three byte-aligned naturals.
        "uplcVersion": ".".join(str(b) for b in data[:3]),
        "builtins": f"protocol 11 or later: {newer}" if newer else "protocol 10 or earlier",
    }


def shape(blueprint):
    definitions = blueprint.get("definitions", {})
    validators = {}
    for suffix in CONSUMED:
        found = [
            v for v in blueprint.get("validators", [])
            if v["title"] == suffix or v["title"].endswith("." + suffix)
        ]
        if len(found) != 1:
            validators[suffix] = f"expected 1 validator titled *{suffix}, found {len(found)}"
            continue
        v = found[0]
        validators[suffix] = {
            "parameters": [normalize(p["schema"], definitions) for p in v.get("parameters", [])],
            "redeemer": normalize(v.get("redeemer", {}).get("schema", {}), definitions),
            "compiledCode": code_shape(v["compiledCode"]),
        }
    return {
        "plutusVersion": blueprint.get("preamble", {}).get("plutusVersion"),
        "validators": validators,
    }


def differences(expected, actual, path="$"):
    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in sorted(set(expected) | set(actual)):
            if key not in actual:
                yield f"{path}.{key}: missing (expected {json.dumps(expected[key])})"
            elif key not in expected:
                yield f"{path}.{key}: unexpected {json.dumps(actual[key])}"
            else:
                yield from differences(expected[key], actual[key], f"{path}.{key}")
    elif isinstance(expected, list) and isinstance(actual, list) and len(expected) == len(actual):
        for i, (e, a) in enumerate(zip(expected, actual)):
            yield from differences(e, a, f"{path}[{i}]")
    elif expected != actual:
        yield f"{path}: expected {json.dumps(expected)}, got {json.dumps(actual)}"


def main():
    args = [a for a in sys.argv[1:] if a != "--write"]
    if len(args) != 1:
        sys.exit(__doc__)
    actual = shape(json.loads(Path(args[0]).read_text()))
    if "--write" in sys.argv[1:]:
        REFERENCE.write_text(json.dumps(actual, indent=2) + "\n")
        return
    expected = json.loads(REFERENCE.read_text())
    diffs = list(differences(expected, actual))
    if diffs:
        print(f"{args[0]} does not match the reference blueprint shape ({REFERENCE.relative_to(ROOT)}):")
        print("\n".join(f"  {d}" for d in diffs))
        print("Off-chain tools would read this blueprint differently than Aiken's.")
        sys.exit(1)
    print(f"{args[0]} matches the reference blueprint shape.")


if __name__ == "__main__":
    main()

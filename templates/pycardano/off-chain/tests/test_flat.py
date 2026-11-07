import cbor2
import pytest
from pycardano import RawPlutusData

from giftcard.contract import OutputReference
from giftcard.flat import apply_params

# (program 1.1.0 (lam x x)), CBOR-wrapped as in a blueprint's compiledCode.
IDENTITY = bytes.fromhex("46010100200101")


def _bytes_param(value):
    return RawPlutusData(value).to_cbor()


CAFE = _bytes_param(bytes.fromhex("cafe"))
SEED = OutputReference(bytes.fromhex("00" * 31 + "ab"), 3).to_cbor()


def test_applies_one_param():
    # The expected programs come from Blaze's `applyParams` (Aiken's UPLC crate).
    assert apply_params(IDENTITY, [CAFE]).hex() == "4d010100320014c10342cafe0001"


def test_applies_params_in_order():
    assert apply_params(IDENTITY, [CAFE, SEED]).hex() == (
        "58390101003320014c010342cafe004c0127d8799f5820"
        "00000000000000000000000000000000000000000000000000000000000000ab"
        "03ff0001"
    )


@pytest.mark.parametrize(
    "params, expected",
    [
        (["beef"], "5201010032488102cafe004c010342beef0001"),
        (["beef", "cafe"], "581901010033248902cafe004c010342beef004c010342cafe0001"),
        (
            ["beef", "cafe", "aa"],
            "58200101003332488102cafe004c010342beef004c010342cafe004c010241aa0001",
        ),
    ],
)
def test_realigns_byte_strings_inside_the_term(params, expected):
    # (program 1.1.0 (lam x (con bytestring #cafe))): shifting the term by the new
    # apply tags changes the padding before #cafe.
    lam_with_bytes = bytes.fromhex("4a010100248902cafe0001")
    encoded = [_bytes_param(bytes.fromhex(p)) for p in params]
    assert apply_params(lam_with_bytes, encoded).hex() == expected


# Every term and constant kind the walker understands: builtin, force, delay, case,
# constr, error, and integer, bytestring, string, unit, list, pair, bool and data
# constants. Encoded by `aiken uplc encode`.
EVERY_TERM = bytes.fromhex(
    "583d010100233333700a330034839c1e910102cafe0049240102686900a4d625eb040a084bded4bd62c101aa008102bbcc00260107d8799f0541ddff000011"
)


@pytest.mark.parametrize(
    "params, expected",
    [
        (
            ["beef"],
            "58440101003233333700a330034839c1e91102cafe0049240102686900a4d625eb040a084bded4bd62c101aa008102bbcc00260107d8799f0541ddff000014c10342beef0001",
        ),
        (
            ["beef", "cafe"],
            "584c01010033233333700a330034839c1e910102cafe0049240102686900a4d625eb040a084bded4bd62c101aa008102bbcc00260107d8799f0541ddff000014c10342beef004c010342cafe0001",
        ),
    ],
)
def test_walks_every_kind_of_term_and_constant(params, expected):
    encoded = [_bytes_param(bytes.fromhex(p)) for p in params]
    assert apply_params(EVERY_TERM, encoded).hex() == expected


def _program(term_bits):
    bits = "00000001" "00000001" "00000000" + term_bits
    bits += "0" * (7 - len(bits) % 8) + "1"
    return cbor2.dumps(int(bits, 2).to_bytes(len(bits) // 8, "big"))


def test_walks_builtins_it_has_no_name_for():
    # (program 1.1.0 (builtin 88)): `uplc` 1.3.3 stops at builtin 86.
    program = _program("0111" "1011000")
    assert apply_params(apply_params(program, [CAFE]), [CAFE]) == apply_params(program, [CAFE, CAFE])


def test_applies_params_to_the_deepest_script_a_tx_can_hold():
    # 16 KB of nested lambdas around a variable: far past Python's recursion limit.
    deep = _program("0010" * 32_760 + "0000" "00000001")
    assert len(apply_params(deep, [CAFE])) > len(deep)


def test_accepts_a_doubly_wrapped_program():
    assert apply_params(bytes.fromhex("47") + IDENTITY, [CAFE]) == apply_params(IDENTITY, [CAFE])


def test_applied_program_can_be_applied_again():
    assert apply_params(apply_params(IDENTITY, [CAFE]), [SEED]) == apply_params(
        IDENTITY, [CAFE, SEED]
    )


def test_splits_params_longer_than_a_flat_chunk():
    long_param = bytes(600)
    applied = apply_params(IDENTITY, [long_param])
    assert long_param not in applied
    # Walking the result raises if the chunks are malformed.
    assert len(apply_params(applied, [CAFE])) > len(applied)


def test_rejects_trailing_bytes():
    with pytest.raises(ValueError, match="Trailing"):
        apply_params(bytes.fromhex("47010100200101ff"), [CAFE])


def test_rejects_a_truncated_program():
    with pytest.raises(ValueError, match="Truncated"):
        apply_params(bytes.fromhex("4301010020"[:8]), [CAFE])

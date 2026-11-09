"""Apply parameters to a compiled UPLC script.

PyCardano has no equivalent of `applyParamsToScript`, and the one Python library
that does (`uplc` 1.3.3) rejects builtins newer than the ones it knows, such as
number 88, which Scalus emits. Applying parameters only requires copying the
program's term behind the new `apply` tags, which this module does by walking the
Flat encoding without decoding it, so it needs no list of builtins. The copy can't
be verbatim: Flat byte-aligns byte strings, so shifting the term changes their
padding.

See the Plutus Core specification, appendix "Flat serialisation".
"""

from collections.abc import Sequence

import cbor2

_LIST_TYPE, _PAIR_TYPE, _APPLY_TYPE, _DATA_TYPE = 5, 6, 7, 8
_INTEGER_TYPE, _BYTESTRING_TYPE, _STRING_TYPE, _UNIT_TYPE, _BOOL_TYPE = 0, 1, 2, 3, 4

_APPLY_TERM = "0011"
_DATA_CONSTANT_PREFIX = "0100" + "1" + f"{_DATA_TYPE:04b}" + "0"


class _TermCopier:
    """Walks a program's bits, held as a string of 0s and 1s, copying its term."""

    def __init__(self, program: bytes):
        self.bits = "".join(f"{byte:08b}" for byte in program)
        self.pos = 0
        self.copied_to = 0
        self.output: list[str] = []
        self.output_size = 0

    def take(self, count: int) -> int:
        if self.pos + count > len(self.bits):
            raise ValueError("Truncated UPLC program")
        value = int(self.bits[self.pos : self.pos + count], 2)
        self.pos += count
        return value

    def skip_natural(self) -> None:
        while self.take(8) & 0x80:
            pass

    def skip_filler(self) -> None:
        while self.take(1) == 0:
            pass

    def start_output(self, prefix: str) -> None:
        self.output = [prefix]
        self.output_size = len(prefix)
        self.copied_to = self.pos

    def _emit(self, bits: str) -> None:
        self.output.append(bits)
        self.output_size += len(bits)

    def flush(self) -> None:
        self._emit(self.bits[self.copied_to : self.pos])
        self.copied_to = self.pos

    def skip_byte_chunks(self) -> None:
        self.flush()
        self.skip_filler()
        self.copied_to = self.pos
        self._emit("0" * (7 - self.output_size % 8) + "1")
        while True:
            length = self.take(8)
            if length == 0:
                return
            self.pos += length * 8

    def read_type(self, tags: list[int]) -> tuple:
        tag = tags.pop(0)
        if tag != _APPLY_TYPE:
            return (tag,)
        applied = tags[0]
        if applied == _APPLY_TYPE and tags[1] == _PAIR_TYPE:
            tags[:2] = []
            return (_PAIR_TYPE, self.read_type(tags), self.read_type(tags))
        if applied == _LIST_TYPE:
            tags.pop(0)
            return (_LIST_TYPE, self.read_type(tags))
        raise ValueError(f"Unsupported UPLC constant type application: {applied}")

    def skip_value(self, constant_type: tuple) -> None:
        tag = constant_type[0]
        if tag == _INTEGER_TYPE:
            self.skip_natural()
        elif tag in (_BYTESTRING_TYPE, _STRING_TYPE, _DATA_TYPE):
            self.skip_byte_chunks()
        elif tag == _BOOL_TYPE:
            self.pos += 1
        elif tag == _LIST_TYPE:
            while self.take(1):
                self.skip_value(constant_type[1])
        elif tag == _PAIR_TYPE:
            self.skip_value(constant_type[1])
            self.skip_value(constant_type[2])
        elif tag != _UNIT_TYPE:
            raise ValueError(f"Unsupported UPLC constant type: {tag}")

    def skip_constant(self) -> None:
        tags = []
        while self.take(1):
            tags.append(self.take(4))
        self.skip_value(self.read_type(tags))

    def skip_term(self) -> None:
        # Iterative: compiled scripts nest deeper than Python's recursion limit.
        # Each stack entry is a pending term ("term") or term list ("list").
        stack = ["term"]
        while stack:
            if stack.pop() == "list":
                if self.take(1):
                    stack += ["list", "term"]
                continue
            tag = self.take(4)
            if tag == 0:  # var
                self.skip_natural()
            elif tag in (1, 2, 5):  # delay, lam, force
                stack.append("term")
            elif tag == 3:  # apply
                stack += ["term", "term"]
            elif tag == 4:  # const
                self.skip_constant()
            elif tag == 6:  # error
                pass
            elif tag == 7:  # builtin
                self.pos += 7
            elif tag == 8:  # constr
                self.skip_natural()
                stack.append("list")
            elif tag == 9:  # case
                stack += ["list", "term"]
            else:
                raise ValueError(f"Unknown UPLC term tag: {tag}")


def _with_filler(bits: str) -> str:
    return bits + "0" * (7 - len(bits) % 8) + "1"


def _append_data_constant(bits: str, cbor: bytes) -> str:
    bits = _with_filler(bits + _DATA_CONSTANT_PREFIX)
    chunks = bytearray()
    for start in range(0, len(cbor), 255):
        chunk = cbor[start : start + 255]
        chunks += bytes([len(chunk)]) + chunk
    chunks.append(0)
    return bits + "".join(f"{byte:08b}" for byte in chunks)


def _unwrap(compiled_code: bytes) -> bytes:
    """Strip the CBOR byte-string wrapping(s) around a Flat program."""
    data = compiled_code
    while data and data[0] >> 5 == 2:
        data = cbor2.loads(data)
    return data


def apply_params(compiled_code: bytes, params: Sequence[bytes]) -> bytes:
    """Apply CBOR-encoded Plutus Data `params` to a blueprint `compiledCode`.

    Returns the applied program wrapped once in a CBOR byte string, which is the
    form `PlutusV3Script` expects.
    """
    copier = _TermCopier(_unwrap(compiled_code))
    for _ in range(3):  # major.minor.patch version
        copier.skip_natural()
    copier.start_output(copier.bits[: copier.pos] + _APPLY_TERM * len(params))

    copier.skip_term()
    copier.flush()
    copier.skip_filler()
    if copier.pos != len(copier.bits):
        raise ValueError("Trailing bytes after UPLC program")

    bits = "".join(copier.output)
    for param in params:
        bits = _append_data_constant(bits, param)
    bits = _with_filler(bits)
    return cbor2.dumps(int(bits, 2).to_bytes(len(bits) // 8, "big"))

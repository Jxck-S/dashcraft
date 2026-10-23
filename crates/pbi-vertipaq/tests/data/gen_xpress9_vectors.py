"""Regenerate the synthetic XPress9 test vectors in this directory.

The plaintext is synthetic (deterministic xorshift32 word salad; no customer
data). It is compressed with the MIT-licensed Microsoft XPress9 encoder via
the `xpress9` PyPI package (https://github.com/Hugoberry/xpress9-python), and
framed the way Power BI frames a PBIX `DataModel` stream (see
../../FORMAT-NOTES.md). The Rust tests regenerate the same plaintext with an
identical generator and compare byte for byte.

Usage: python gen_xpress9_vectors.py   (needs `pip install xpress9`)
"""
import struct
from pathlib import Path

from xpress9 import Xpress9

WORDS = [b"alpha", b"beta", b"gamma", b"delta", b"Outfitters", b"Trail Packs",
         b"Shipped", b"VertiPaq", b"Acme", b"zz"]
SINGLE_SIG = "This backup was created using XPress9 compression.".encode("utf-16le") + b"\0\0"
MULTI_SIG = "This backup was created using multithreaded XPrs9.".encode("utf-16le") + b"\0\0"


def text(seed: int, n: int) -> bytes:
    x = seed
    out = bytearray()
    while len(out) < n:
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        out += WORDS[x % len(WORDS)]
        out += b"\n" if (x >> 8) % 5 == 0 else b" "
    return bytes(out[:n])


def single_threaded_chunks():
    c0 = text(1, 3000)
    c1 = b"A" * 5000 + text(2, 500)
    c2 = text(3, 2500) + c0[100:1100]
    return [c0, c1, c2]


def multi_threaded_groups():
    # prefix: 1 group x 1 chunk; main: 2 groups x 2 chunks
    prefix = [[text(11, 1500)]]
    main = [[text(21, 1200), text(22, 900)], [text(31, 1300), text(32, 800) + b"B" * 600]]
    return prefix, main


def skewed(n: int = 8000, seed: int = 99) -> bytes:
    """32 common byte values in xorshift-random order plus a 9-symbol tail with counts
    1, 1, 2, 4, ..., 128 scattered at random positions.

    Random 5-bit symbols leave little for LZ77 to match, and the exponential tail forms
    a chain in the Huffman tree, so the rarest symbols need codewords longer than 12
    bits (the Rust decoder's fast-table width).
    """
    x = seed

    def nxt():
        nonlocal x
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        return x

    data = bytearray(0x40 + (nxt() >> 7) % 32 for _ in range(n))
    for k in range(9):
        for _ in range(max(1, 1 << (k - 1)) if k else 1):
            data[nxt() % n] = 0x80 + k
    return bytes(data)


def frame(enc, chunk):
    comp = enc.compress(chunk, len(chunk))
    return struct.pack("<II", len(chunk), len(comp)) + comp


def main():
    here = Path(__file__).parent
    enc = Xpress9()  # one encoder session across all chunks (single-threaded layout)
    st = bytearray(SINGLE_SIG)
    for c in single_threaded_chunks():
        st += frame(enc, c)
    (here / "xpress9_single_threaded.bin").write_bytes(st)

    prefix, main_groups = multi_threaded_groups()
    mt = bytearray(MULTI_SIG)
    mt += struct.pack("<5Q", 2, 1, len(prefix), len(main_groups), 4096)
    for group in prefix + main_groups:
        enc = Xpress9()  # every group is an independent XPress9 session
        for c in group:
            mt += frame(enc, c)
    (here / "xpress9_multi_threaded.bin").write_bytes(mt)

    sk = bytearray(SINGLE_SIG) + frame(Xpress9(), skewed())
    (here / "xpress9_skewed.bin").write_bytes(sk)


if __name__ == "__main__":
    main()

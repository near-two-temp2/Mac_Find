"""Cling-style 64-bit character bitmask for O(1) candidate pre-filtering.

Each set bit records that a given character *class* appears somewhere in the
(lowercased) string. Bit layout is shared with the other language
implementations in this repo (see ``open-source-analysis.md`` §3.3 and the Go
peer at ``Road_C/Src_GO/internal/bitmask/bitmask.go``), so indexes stay
conceptually comparable across languages:

    Bits 0-25:  letters a-z
    Bits 26-35: digits 0-9
    Bit 36:     '.'
    Bit 37:     '-'
    Bit 38:     '_'

A query can only match an entry if every bit of the query mask is also set in
the entry mask, i.e. ``entry & query == query``. One ``AND`` discards the vast
majority of non-matching entries.
"""

from __future__ import annotations

import numpy as np

BIT_DOT = 36
BIT_DASH = 37
BIT_UNDERSCORE = 38

# Precomputed byte -> mask table (256 entries). Index by the *lowercased* byte.
_BYTE_MASK = np.zeros(256, dtype=np.uint64)
for _c in range(ord("a"), ord("z") + 1):
    _BYTE_MASK[_c] = np.uint64(1) << np.uint64(_c - ord("a"))
for _c in range(ord("0"), ord("9") + 1):
    _BYTE_MASK[_c] = np.uint64(1) << np.uint64(26 + (_c - ord("0")))
_BYTE_MASK[ord(".")] = np.uint64(1) << np.uint64(BIT_DOT)
_BYTE_MASK[ord("-")] = np.uint64(1) << np.uint64(BIT_DASH)
_BYTE_MASK[ord("_")] = np.uint64(1) << np.uint64(BIT_UNDERSCORE)

# Fold uppercase ASCII onto lowercase so callers with mixed-case input still map
# to the same class bit.
for _c in range(ord("A"), ord("Z") + 1):
    _BYTE_MASK[_c] = _BYTE_MASK[_c + (ord("a") - ord("A"))]

del _c


def of_bytes(data: bytes) -> np.uint64:
    """Return the character-class mask for ``data`` (a lowercased path/query)."""
    if not data:
        return np.uint64(0)
    arr = np.frombuffer(data, dtype=np.uint8)
    # Bitwise-OR of the per-byte masks. ``np.bitwise_or.reduce`` over the lookup.
    return np.bitwise_or.reduce(_BYTE_MASK[arr])


def of_str(s: str) -> np.uint64:
    """Return the character-class mask for ``s`` (lowercased on the fly)."""
    return of_bytes(s.lower().encode("utf-8", "ignore"))


def matches(entry: int, query: int) -> bool:
    """True if an entry could contain every character class the query needs."""
    return (int(entry) & int(query)) == int(query)

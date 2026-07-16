"""fzf-style fuzzy scoring over lowercased byte strings.

A compact port of the Cling scoring model (``open-source-analysis.md`` §3.4):
anchor on the first pattern character, greedily match forward, tighten
backward, then score with bonuses for word boundaries and contiguity and
penalties for gaps. Scores are only *relative* — higher is better — so exact
constants need not match Cling's, only their spirit.

On top of the fuzzy scorer sits :func:`rank_score`, which is what the engine
actually calls. It layers big, deterministic tiers on top of the fuzzy value so
that **exact / substring matches always outrank scattered subsequence noise**
(the c-tauri behaviour we are matching: querying ``temp_test`` puts the real
``temp_test`` directory at position 1, not ``vscode_pytest`` etc.).

Everything works on ``bytes`` that are already lowercased, matching how the
index stores paths and how the query is normalised in :mod:`macfind_c.engine`.
"""

from __future__ import annotations

from typing import Optional, Tuple

# Scoring weights (see §3.4 "评分细节").
SCORE_MATCH = 16
SCORE_CONSECUTIVE = 4
FIRST_CHAR_MULTIPLIER = 2
BONUS_BOUNDARY_CAMEL = 7
BONUS_BOUNDARY_SPACE = 8
BONUS_BOUNDARY_SEP = 9
PENALTY_GAP_START = -3
PENALTY_GAP_EXTEND = -1

# --- Match-quality tiers (dominate the fuzzy value) ----------------------- #
# These are large relative to any realistic fuzzy score (a long path scores at
# most a few hundred) so the *kind* of match, not fuzzy incidentals, decides the
# order. This is what makes an exact basename hit beat every scattered hit.
TIER_EXACT = 1_000_000      # basename == query
TIER_PREFIX = 500_000       # basename starts with query
TIER_SUBSTRING = 250_000    # basename contains query (contiguous)
TIER_SUBSEQUENCE = 0        # query is only a scattered subsequence

_SEPARATORS = frozenset(b"/\\._- ")


def _boundary_bonus(prev: Optional[int], cur: int) -> int:
    """Bonus for ``cur`` starting a new word given the preceding byte ``prev``."""
    if prev is None:
        return BONUS_BOUNDARY_SEP  # start of string behaves like after a sep
    if prev == 0x20:  # space
        return BONUS_BOUNDARY_SPACE
    if prev in _SEPARATORS:
        return BONUS_BOUNDARY_SEP
    # camelCase: lowercase/digit followed by an uppercase in the *original* text.
    # We operate on lowered bytes, so approximate: a letter after a digit or a
    # digit after a letter also reads as a soft boundary.
    prev_alpha = 0x61 <= prev <= 0x7A
    cur_digit = 0x30 <= cur <= 0x39
    prev_digit = 0x30 <= prev <= 0x39
    cur_alpha = 0x61 <= cur <= 0x7A
    if (prev_alpha and cur_digit) or (prev_digit and cur_alpha):
        return BONUS_BOUNDARY_CAMEL
    return 0


def score(pattern: bytes, text: bytes) -> Optional[Tuple[int, int, int]]:
    """Score ``pattern`` against ``text`` (both lowercased).

    Returns ``(score, start, end)`` of the best contiguous-ish match window, or
    ``None`` if ``pattern`` is not a subsequence of ``text``. An empty pattern
    scores 0 at the origin (used to list "everything").
    """
    if not pattern:
        return (0, 0, 0)
    if not text or len(pattern) > len(text):
        return None

    first = pattern[0]
    best: Optional[Tuple[int, int, int]] = None

    # Anchor enumeration: try every occurrence of the first pattern byte.
    anchor = text.find(first, 0)
    anchors_tried = 0
    while anchor != -1 and anchors_tried < 32:
        anchors_tried += 1
        s = _score_from_anchor(pattern, text, anchor)
        if s is not None and (best is None or s[0] > best[0]):
            best = s
        anchor = text.find(first, anchor + 1)

    return best


def _score_from_anchor(
    pattern: bytes, text: bytes, anchor: int
) -> Optional[Tuple[int, int, int]]:
    """Greedy forward match of ``pattern`` in ``text`` starting at ``anchor``."""
    total = 0
    pi = 0
    ti = anchor
    prev_matched = -2  # index of previously matched text byte
    in_gap = False
    n = len(text)
    plen = len(pattern)

    while pi < plen and ti < n:
        if text[ti] == pattern[pi]:
            prev_byte = text[ti - 1] if ti > 0 else None
            # A word-boundary bonus only counts when we arrive at the boundary
            # contiguously. If we had to skip characters to reach it, the gap is
            # already penalised and the boundary must not also reward the jump —
            # otherwise a scattered "c_a_t" would beat a contiguous "cat".
            if pi == 0 or not in_gap:
                bonus = _boundary_bonus(prev_byte, text[ti])
            else:
                bonus = 0
            gain = SCORE_MATCH + bonus
            if prev_matched == ti - 1:
                gain += SCORE_CONSECUTIVE
            if pi == 0:
                gain *= FIRST_CHAR_MULTIPLIER
            total += gain
            prev_matched = ti
            pi += 1
            ti += 1
            in_gap = False
        else:
            total += PENALTY_GAP_EXTEND if in_gap else PENALTY_GAP_START
            in_gap = True
            ti += 1

    if pi != plen:
        return None
    return (total, anchor, prev_matched + 1)


def rank_score(
    pattern: bytes, path: bytes, bn_start: int
) -> Optional[int]:
    """Final ranking score for one index entry — the engine's real hot path.

    ``pattern`` and ``path`` are lowercased bytes; ``bn_start`` is the byte
    offset of the basename inside ``path`` (index of the char after the last
    ``/``). Returns an integer where **higher is better**, or ``None`` if the
    query is not even a subsequence of the path (drop the entry).

    Ranking policy, in order of dominance (this is the c-tauri parity fix):

    1. Basename **exact / prefix / substring** matches get a huge tier bonus so
       they always beat scattered subsequence hits — a query ``temp_test``
       ranks the real ``temp_test`` folder at the top, never ``vscode_pytest``.
    2. Basename matches are always preferred over matches that only appear
       deeper in the path (``+128`` basename boost vs a full-path fuzzy hit).
    3. Within a tier, the fzf fuzzy value orders by contiguity / boundaries.
    4. Shorter basenames win ties (the more-specific hit) — handled by the
       engine's final sort; here we only fold in a tiny length nudge so a
       whole-word exact match isn't diluted by trailing chars.
    """
    if not pattern:
        return 0

    basename = path[bn_start:]

    # --- Basename tiers (dominant) ------------------------------------------
    if basename == pattern:
        # Exact whole-name match: the strongest possible signal.
        return TIER_EXACT

    pos = basename.find(pattern)
    if pos == 0:
        # Prefix match. Reward tighter fits (shorter tail) slightly.
        tail = len(basename) - len(pattern)
        return TIER_PREFIX + max(0, 256 - tail)
    if pos > 0:
        # Contiguous substring somewhere in the basename. Earlier = better,
        # and a boundary-aligned occurrence (after / . - _ space) reads as a
        # word start and should edge out a mid-word one.
        prev = basename[pos - 1]
        boundary = prev in _SEPARATORS
        return (
            TIER_SUBSTRING
            + (64 if boundary else 0)
            + max(0, 256 - pos)  # earlier occurrence ranks higher
        )

    # --- Fuzzy subsequence on the basename ----------------------------------
    bn_hit = score(pattern, basename)
    if bn_hit is not None:
        # Basename subsequence: still clearly better than a full-path-only hit.
        return TIER_SUBSEQUENCE + 128 + bn_hit[0]

    # --- Fall back to the whole path (needed for queries containing '/') ----
    # A substring of the full path (e.g. "src/main") is contiguous and should
    # sit just under a basename subsequence.
    ppos = path.find(pattern)
    if ppos >= 0:
        return TIER_SUBSEQUENCE + 64 + max(0, 256 - ppos)

    path_hit = score(pattern, path)
    if path_hit is not None:
        return TIER_SUBSEQUENCE + path_hit[0]

    return None

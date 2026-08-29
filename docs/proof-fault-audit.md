# Fault audit of the Kani proof harnesses

A proof that passes tells you nothing until you know it can fail. This document
records what happens when deliberately wrong geometry is fed to the harnesses in
`checkers-core/src/geometry.rs`.

Method: inject one wrong line, run `scripts/verify-proofs.sh`, record which
harnesses fail, restore. The driver is throwaway (`perl -0pi` over a saved copy,
restored by a shell trap); the results are not, so they live here.

The number that matters is not "did the suite go red" but **which** harnesses
went red, and whether that matches the claim each one advertises in its name. A
fault caught by only one harness is a single point of failure. A fault caught by
none is a hole.

## Results

| # | Injected fault | Caught by |
|---|---|---|
| F1 | camp triangle points inward (the original draft's bug) | 3 |
| F2 | `rotate60` replaced by a reflection | 7 |
| F3 | `rotate60` replaced by $-\mathrm{id}$ | 4 |
| F4 | `HEX_RADIUS` 4 → 5 | 6 |
| F5 | drop $\lvert q+r\rvert \le 4$ (hexagon → rhombus) | 1 → **2** |
| F6 | camp base column at $q=4$, overlapping the hexagon | 3 |
| F7 | `in_camp` rotates forward, giving $C_i = R^{-i}(C_0)$ | 1 |
| F8 | `jump_dest` displaces by $d$ instead of $2d$ | 1 |
| F9 | `Dir::Se` given the wrong $\Delta q$ | 2 |
| F10 | `opposite()` maps `E` to itself | 1 |
| F11 | `rotate_n` uses `n % 7` | 1 |
| F12 | camp reshaped, still 10 holes, still disjoint | 2 |

Every fault was caught. F5 is the one that found a real weakness.

## What F5 exposed

Dropping the $\lvert q+r\rvert \le 4$ constraint turns $H_4$ into an 81-hole
rhombus. It should be caught everywhere, and was caught almost nowhere:

- It **swallows $C_1$ and $C_4$ whole** — 10 holes each — while leaving $C_0$
  untouched. `hex_and_base_camp_are_disjoint` quantified only over $C_0$, so it
  passed.
- The board still has **exactly 121 holes**: the 20 the rhombus gains are
  precisely the 20 it absorbs from those two camps. The board-size clause of the
  cardinality proof passed.
- The rhombus is centrally symmetric, so $V = -V$ passed.

Only the `hex == 61` clause of `camps_and_hexagon_are_populated` caught it.

The harness was named `CC-GEO-HEX-CAMP-DISJOINT` and its law states
$H_4 \cap C_i = \varnothing$ for every $i$ — but the harness had drifted to
checking $C_0$ alone. It is now `hex_and_camps_are_disjoint`, quantified over a
symbolic camp index like `distinct_camps_are_disjoint` already was, which takes
F5 from 1 catcher to 2.

Worth noting: the *runtime* law `CC-GEO-DISJOINT` had always quantified over all
six camps, so it would have caught this board. The two layers are not redundant,
and the gap was in the proof layer alone.

## Corrections to predictions

Two results contradicted what I expected, and both were my error rather than the
harnesses':

- **F2 was predicted to be caught by one harness, and was caught by seven.** I
  labelled `(q,r) \mapsto (r, -q-r)` as $R^{-1}$, reasoning that an inverse
  rotation preserves order 6 and $R^3 = -\mathrm{id}$. It is not the inverse —
  that is $(q+r, -q)$ — it is a reflection, which has order 2 and fails
  $R^3 = -\mathrm{id}$ outright. The harnesses were right; the label was wrong.
- **F12 was expected to defeat cardinality counting, and does.** A 10-hole
  region of a different shape passes every count and every disjointness check.
  It is caught only by `base_camp_columns_decrease_outward`, which pins the
  4-3-2-1 column profile. That harness is a genuine single point of failure for
  shape, and deliberately so — it exists because F1 originally escaped all 11
  harnesses for exactly this reason.

## Single-catcher faults

F7, F8, F10, F11 each rest on one harness. Unlike F12 this is not a concern:
each is the harness that directly states the property (`distinct_camps_are_disjoint`
for camp placement, `jumped_hole_is_the_midpoint` for jump displacement,
`direction_opposite_negates` for the involution, `rotation_has_order_exactly_six`
for the modulus). A property asserted in exactly one place is fine when that
place is the one that names it.

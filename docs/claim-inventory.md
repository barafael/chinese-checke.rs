# Claim inventory — chapters 6–15

The outcome of the formalization pass the README used to advertise as
outstanding. Method: every normative claim in the generated specification's
prose (chapters 6–15) was extracted and mapped to the law that checks it;
anything with no law was a candidate gap. The originals in `docs/superseded/`
were used to spot-check that the generated prose had not dropped claims in
transcription.

## Result: no formalisable claim is without a law

Chapters 1–5 (geometry) hold 15 laws; chapters 6–15 hold 28; the registry
totals 43, and `checkers-spec-gen --check` keeps the document and the
registry identical. Per chapter:

| Chapter | Claims in prose | Laws |
|---|---|---|
| 6. Players, pieces, initial position | ten pieces each; 60/61 occupancy; initial camps full, hexagon empty; target is the opposite camp | `CC-POS-INITIAL`, `CC-POS-OCCUPANCY`, `CC-POS-PIECES`, `CC-POS-TARGET` |
| 7. Adjacent moves | step to adjacent empty hole; displacement of exactly one | `CC-STEP-LEGAL`, `CC-STEP-DISPLACE` |
| 8. Jumps | cross occupied, land empty; any owner; no capture; lands two away, jumped hole is the midpoint | `CC-JUMP-LEGAL`, `CC-JUMP-ANY-OWNER`, `CC-JUMP-NO-CAPTURE`, `CC-JUMP-DISPLACEMENT` |
| 9. Sequences and reachability | other pieces frozen during the turn (Ω); BFS = closure; routes may revisit holes; one hop at a time; chaining reaches the closure; ending at the origin is not a move | `CC-JUMP-OMEGA`, `CC-JUMP-CLOSURE`, `CC-JUMP-REVISIT`, `CC-TURN-HOP-ONE`, `CC-TURN-HOP-CLOSURE`, `CC-TURN-NO-NULL-MOVE` |
| 10. Move representation | move = kind + origin + destination; routes excluded from identity; one move per destination; generated moves start on own pieces; staged clicks commit to real moves | `CC-MOVE-IDENTITY`, `CC-MOVE-DEDUP`, `CC-MOVE-ONBOARD`, `CC-TURN-STAGED-LEGAL` |
| 11. Applying a move | net effect = route replay | `CC-APPLY-NET` |
| 12. Turn order, passing | cyclic order; blocking is reachable and is not a loss; pass when stuck; all-pass is a draw; a played move resets the pass count | `CC-TURN-CYCLE`, `CC-TURN-BLOCKED`, `CC-TURN-PASS`, `CC-TURN-PASS-RESET` |
| 13. Winning | fill the opposite camp | `CC-WIN-CONDITION` |
| 14. Invariants | moves preserve piece counts and occupancy | `CC-INV-PRESERVED` |
| 15. Variants | camps add no movement rules (explicit choice) | `CC-VAR-CAMP-FREE` |

Claims that are *combinations* of laws are deliberately not given laws of
their own: "the player may stop after any jump" is `CC-TURN-STAGED-LEGAL`
plus `CC-TURN-NO-NULL-MOVE`; "at most one piece per hole" is structural —
the position *is* a function from holes, so there is nothing to check.

## Candidates considered and declined

Two prose claims have no dedicated law. Both were judged decoration:

1. **A turn is a step or a jump chain, never a mixture.** True by
   construction: a move's kind is a two-variant enum, and move generation
   builds each kind from its own rule. A law would restate the type
   definition as a runtime check.
2. **The game ends at the first winning position.** Enforced by
   `Game::play`/`Game::pass` asserting `!is_over()` — a guard, not a
   specification claim about positions or moves, which is what the registry
   is for.

If either ever gains an implementation that could plausibly violate it, it
becomes a law (`CC-TURN-KIND-PURITY`, `CC-WIN-TERMINAL` are reserved-style
IDs); today there is nothing for them to catch that the type system and the
assertions do not already catch.

## Consequence

The README's paragraph "what remains to verify against the originals is the
detail of chapters 6–15" was stale — written before the chapter 6–15 laws
landed. It is corrected in the same commit as this inventory. Future prose
changes to the specification should extend the law registry in the same
commit, which is exactly the discipline the registry was built for.

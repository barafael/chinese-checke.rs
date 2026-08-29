# checkers-model

Naive reference implementation of the Chinese Checkers rules.

Its role in the workspace is to be the **differential model**: a deliberately
simple, obviously-correct implementation that an optimised engine can be tested
against. The normative laws live in `checkers-core`. **This is a prototype**: its job is to keep the specs honest, not to
be a fast engine.

Every non-obvious claim in the prose is an assertion or a `#[test]` here, with
`§` references back to the relevant section.

```sh
cargo test              # 29 tests: geometry, move gen, invariants, pass/draw
cargo run -p checkers-model --bin demo    # prints the star, plays a game, runs reachability search
```

## Layout

| File | Spec sections | Contents |
| --- | --- | --- |
| `src/coord.rs` | §1, §4, §10 | Axial coords, six directions, 60° rotation |
| `src/board.rs` | §§2–6 | Hexagon, camps, star construction, invariants, ASCII render |
| `src/moves.rs` | §§11–12, 16, 19–20 | Step/jump legality, jump BFS, route enumeration, move gen |
| `src/state.rs` | §§8–9, 21, 23, 28 | Occupancy, apply, win test, position invariants |
| `src/game.rs` | §§22, 24–25 | Turn sequencing, pass/draw, blocked-position builder |
| `src/prng.rs` | — | Tiny xorshift, so the crate has no dependencies |

## The three bugs this crate pins down

These were found in the draft specs and are now regression-tested:

1. **Camps must point outward.** The draft's inward-pointing triangle yields ten
   holes per camp and 121 total — so cardinality checks pass — but each camp
   meets the hexagon in *one* hole instead of four, leaving the points detached.
   Caught by the 8-contact invariant; see `board::tests::inward_camp_is_rejected`.

2. **Jump search is over positions, not `(state, position)`.** A turn moves one
   piece and never captures, so occupancy is a function of position *within* a
   turn. The draft's recommended `(state, position)` keying can never fire where
   plain position wouldn't, and so does not terminate. See
   `moves::tests::bfs_agrees_with_exhaustive_path_search`.

3. **A player can have zero legal moves.** The draft asserted this was
   impossible. `game::blocked_position` builds a legal position where a player
   holds all ten pieces yet cannot move — neither a win nor a loss — which is why
   `Game` implements passing and a six-pass draw.

## Prototype shortcuts

- `HashMap`/`HashSet` everywhere; a real engine would use a packed 121-cell array
  and bitboards, and would not `clone()` the board into every `State`.
- `Move::route` is optional and presentational. Move identity is
  `(kind, origin, destination)` per §19, so routes are excluded from `Eq`/`Hash`.
- The demo's solo search uses an **inadmissible** heuristic (it sums hex
  distances, but a jump covers two hexes), so its move count proves reachability
  only — it is not a shortest solution.
- The camp-rule parameterization of §30 (`CampLegal`) is not implemented; this
  crate hardcodes the unrestricted convention.
- Random play does not converge to a win, which is expected: random agents
  wander. Use the demo's search to see the goal actually reached.

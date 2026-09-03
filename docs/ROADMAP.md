# Roadmap — remaining work

Living document; edit freely. Status lines are dated and every item is a
checkbox so progress is visible at a glance.

**Status 2026-09-03** (`2493158` + uncommitted menu-bg work): the core is
green — 43 laws, all play modes (solo, human-vs-AI, Watch 2 Bots + live menu
background, hotseat, networked multiplayer), two board styles, staged steps
and jumps, game-over statistics, wasm deploy, full CI. What follows is depth,
not repair.

## Phase 0 — Housekeeping

- [x] Commit the menu-background feature (`menu_bg.rs` new; `board_view.rs`,
  `lib.rs`, `main.rs`, `menu.rs` changed) together with the parallel
  session's free-orbit 3D camera — the files are entangled and the gates were
  green. `moves.log` stays untracked.
- [x] Drop `stash@{0}` and `stash@{1}`: the rebase-conflict audit verified
  `stash@{0}` reverse-applies cleanly onto HEAD and `stash@{1}`'s features
  landed in evolved form (`Game::for_players`/`compose`, `apply_seats`,
  `turn_order_skips_vacant_seats`).
- [x] Append this session's record to `docs/session-details.md`.

## Phase 1 — Documentation honesty

- [x] README "Playing" says "Steps commit immediately" — stale since
  step-staging shipped. Steps now also wait for Enter. Rewrite the paragraph
  and the input table.
- [x] Inventory every global key from `handle_keys` (`U` `R` `T` `A` `V`
  `Escape`, Enter/Backspace) and document each accurately; mention Watch
  2 Bots, the live menu background, and the statistics screen.

## Phase 2 — Resign

- [x] Core-honest: `Outcome::Resigned(Player)` variant — a concession is a
  fact about the round, so it lives in the engine next to `Winner`/`Draw`.
  `Game::resign(p)` refuses unseated seats and finished games; the position
  is deliberately untouched. Core tests cover the contract.
- [x] **Button only — no key binding** (decided). Gated by `may_act()`,
  inert once the game is over, and hidden in networked games until a
  concession can cross the wire.
- [x] The status line, turn indicator, game-over card, and move log report
  the resignation; hotseat gives up the seat to move, a pinned player
  resigns even off turn.

## Phase 3 — AI difficulty

- [x] **Strength levels 1–5** (decided): five presets over `AiConfig`
  (wall-clock budget × max depth) via `AiConfig::strength(level)`. Level 3 is
  exactly the default tuning; 1–2 cut the budget steeply so the difference is
  felt within a move, 4–5 pay seconds for deeper play. Every level keeps the
  depth-cap safety net, so real games still finish.
- [x] Radio-style strength row (1–5) on the hotseat panel; the engine is
  rebuilt at the chosen strength when the game is dealt — which also means a
  watched race runs at the chosen strength. The menu background keeps its own
  30 ms decorative engine.
- [x] Tests: the five presets are distinct, level 3 equals the default, and
  out-of-range levels clamp instead of panicking.

## Track A — Spec formalization, chapters 6–15 (the long track)

The project's stated purpose (README:73-77): chapters 6–15 prose detail is
stated but not yet formalised as laws.

- [x] Claim inventory: every normative claim in chapters 6–15 maps to a law
  (`docs/claim-inventory.md`). The prose's two uncovered statements — turn
  kind purity and win terminality — are true by construction or by guard,
  and are recorded there as declined candidates with their reasons. The
  README's "chapters 6–15 remain" paragraph was stale and is fixed.
- [x] Chapter-by-chapter slices: nothing to formalise — closed by the
  inventory. Any future prose change extends the registry in the same
  commit.
- [ ] Mutation-test the riskiest *existing* laws beyond the 14/14 exercise
  already recorded; `scripts/verify-proofs.sh` whenever geometry or move
  generation changes.

## Phase 4 — Save / resume / replay

Storage decided: **real files on native, localStorage on wasm**. Records use
the **`.cchkrs`** extension (decided).

- [ ] Record format: versioned text header (format version, seating,
  players, result) followed by the sequenced move list, reusing
  `checkers-net`'s `WireMove` forms so replay rides the same path the net
  tests exercise.
- [ ] Save: native file dialog (`rfd` or equivalent); wasm localStorage;
  clipboard copy as the universal fallback.
- [ ] Resume: parse, then replay through the sequencing path with the law
  audit applied per move — a stale or illegal record is rejected honestly.
- [ ] Replay viewer: step and autoplay controls, paced like `AiPace`.
- [ ] Tests: save→load round-trip identity (position, turn, outcome); a
  corrupted record is rejected with a readable message.

## Phase 5 — Networked AI seats

- [ ] Host-only ownership: the lobby roster gains AI-seat marks; the host's
  engine drives those seats through the existing outbox → wire path (one
  sequencing path, no privileged moves — `ai_take_turn` already pushes to the
  outbox).
- [ ] Document the edge rule: no host migration — an AI seat stalls if the
  host leaves.
- [ ] Net tests: AI moves reach the guest; a guest cannot drive an AI seat.

## Phase 6 — Move animation

- [ ] Tween each hop over ~120 ms, driven by Bevy's `Time` (wasm-safe, same
  discipline as `AiPace`). Applies to human hops, AI hops, and replay.
- [ ] `sync_pieces` snaps only on commit, new deal, or style switch — it must
  never fight an in-flight tween.
- [ ] Purely visual: no law impact. Headless test — the tween lands exactly on
  `coord_to_world(target)`.

## Phase 7 — Touch input (spike first)

- [ ] Spike on the deployed Pages build with a real tablet/phone: does Bevy's
  tap→click synthesis already cover select/confirm/cancel?
- [ ] Pinch-zoom for the amlah orbit camera; one-finger drag orbits.
- [ ] Viewport/meta tags in `index.html`; hit-target sizing on small screens.

## Phase 8 — Audio

- [ ] `bevy_audio`; small sounds for hop, commit, cancel, win, resign.
  Decide in a spike: generated by a checked-in script versus committed
  assets.
- [ ] Mute key. On wasm, start audio only after the first user gesture
  (autoplay policy) — the first menu click qualifies.

## Definition of done (every slice)

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D
warnings` · `cargo test --workspace` · `cargo doc --workspace --no-deps` ·
spec-gen `--check` + `--check-registry` · wasm check for `checkers-bevy` ·
Kani when rules code changes · commit everything green.

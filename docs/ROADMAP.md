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

- [x] Record format: versioned text header (format version, seating, engine
  seats) followed by the moves in play order as route-free `WireMove` lines —
  the position is *derived*, not stored, so a record cannot smuggle the game
  anywhere the rules disallow (`checkers-bevy/src/record.rs`).
- [x] Save: native file dialog (`rfd`); wasm localStorage. Clipboard fallback
  turned out unnecessary — both platforms are covered; revisit if a browser
  ever refuses storage.
- [x] Resume: parse, then replay through `WireMove::resolve` against the legal
  moves of each position, law audit per move — a stale, forged, or corrupted
  record is refused with a readable fault. Auto-passes re-derive exactly.
- [x] Replay viewer: a Replay button walks a saved record — arrows step a
  ply either way, Space toggles one-second autoplay, Escape hands the board
  back to the round's end. The on-screen session is derived at the cursor via
  `Session::resumed_prefix`, so every rendering system works unchanged, the
  law audit holds at every ply, and forward steps animate through the
  ordinary flight machinery (a viewer session controls no seat, so every
  move is news). While it is up, the play keys, clicks, controls, engine,
  and round clock all stand down.
- [x] Tests: round-trip identity (position, turn, outcome, move counts);
  corrupted records (header, seating, move lines, count) refused; an illegal
  recorded move rejected on replay; resumed position passes the audit.

## Phase 5 — Networked AI seats

- [x] Host-only ownership: the lobby's Add engine button seats an engine as
  a roster entry no peer commands; it joins, reads as ready, and takes a
  camp in join order like any player. Only the sequencing authority's
  engine plays those camps, and its moves reach every peer as ordinary
  sequenced moves through the one path every move takes — a guest holding
  the same roster cannot double-drive a seat.
- [x] Edge rule documented: no host migration — an engine belongs to the
  session that seated it. Solo play strips engine seats (that game lives in
  the hotseat panel).
- [x] Tests: engine seating, capacity refusal at the seating's count,
  join-order camp binding, and host-only driving.

## Phase 6 — Move animation

- [x] Delivered by the replay module: the opponent's last turn flies hop by
  hop with the route left as a grey trace, driven by Bevy's time.
- [x] Animating the human's own hops: **declined, deliberately.** Hotseat
  already animates own moves (no seat is pinned, so every move is replayed);
  in solo play, own-move flights would delay the player's own feedback loop
  for an action they just took and can see. The replay viewer gets flights
  for free by the same rule. Revisit only if a seat's own moves ever need to
  be *announced* rather than performed.

## Phase 7 — Touch input

- [x] One-finger drag orbits the amlah board; a two-finger pinch works the
  radius (a touchscreen has neither a right button nor a wheel). Taps click
  the board through their own path: on the web canvas winit prevents the
  browser's emulated mouse events, so a finger lifting within a flick of
  where it landed counts as a click, and a drag — which orbits — travels too
  far to qualify. The viewport meta and `touch-action: none` were already in
  place.
- [ ] The manual spike remains: how it *feels* on real glass (tap threshold,
  hit-target sizing) can only be judged on a device.

## Phase 8 — Audio

- [x] Delivered with Bevy's built-in `Pitch` source: the backend synthesizes
  each sine tone itself, so the "checked-in script versus committed assets"
  question dissolved — there are no files at all and no decoder features.
  A hop ticks, a committed turn settles, an abandoned one sinks, a win
  rings, a resignation falls. Hop and commit are watched on the session, so
  clicks, keys, and engines all sound alike.
- [x] `M` mutes, in every state. On wasm the first click is both the gesture
  the browser waits for and the first tone, so autoplay policy is met by
  construction.

## Definition of done (every slice)

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D
warnings` · `cargo test --workspace` · `cargo doc --workspace --no-deps` ·
spec-gen `--check` + `--check-registry` · wasm check for `checkers-bevy` ·
Kani when rules code changes · commit everything green.

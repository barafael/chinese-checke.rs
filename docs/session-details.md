# Session details

## 2026-09-02 — review, proofs, mutations, multiplayer fixes (main session)

- **Review & simplification.** Audit shrunk to reachable faults; dead API
  (`all_sorted`, `by_reading_order`) and `proptest` removed; shared `Xorshift`
  in core; `camp_of`/`distance`/hex-mapping algebra simplified; camp holes
  cached (`OnceLock`); `checkers-bevy` split into lib+bin (headless tests use
  the real mapping); lobby Hello-flood fixed; roster broadcast deduped.
- **Differential harness** (`checkers-model/tests/differential.rs`): 4 tests
  play core vs model in lockstep — geometry, move sets, jump closures, win
  flags, outcomes. All agree; the crates share no code.
- **Mutation exercise** (14 rule breaks × Kani + laws): geometry mutations
  caught by proofs (often 7/14 harnesses); move-generation mutations caught
  only by the law registry (proofs are geometry-only by design); registry is
  fail-fast and link-ordered. One **survivor** — the pass-counter reset —
  became **`CC-TURN-PASS-RESET`** (43 laws); the mutation now dies at it.
  Kani 0.67 installed: baseline and post-change 14/14 verify.
- **Toolchain**: `rust-toolchain.toml` pins stable — recent nightlies fail
  inside `bevy_render` and masquerade as broken workspace code.
- **Clippy/idiom**: `DrawContext` (`SystemParam`) for the render-resource
  cluster; per-player (not per-piece) materials; test-only helper moved into
  `mod tests`. rustdoc `-D warnings` gate kept green.
- **Two-player fix**: seats were bound to join-order slots (0,1) while the Two
  seating plays camps {0,3} — nobody controlled camp 3, hence the eternal
  "Waiting for player 3". Now `assign_seating` binds slots to camps at start,
  `Seating::game` composes over seated players only, solo starts unbind seats,
  and shared starts need a seat per camp. Three regression tests.
- **Board styles**: Classic/Amlah visualization (`V`) via `board_style` +
  `board_amlah`; `cargo doc` requires `--no-deps` (KaTeX header path is
  relative; README fixed).
- **CI**: one fmt-only failure, fixed from an isolated worktree against the
  pushed code while the other session held the main tree mid-refactor.
- **Protocol note**: the parallel session landed commits repeatedly during this
  session; every push was rebased onto it, and the stash-pop conflict in the
  main tree was resolved by committing the coherent working state.

## 2026-09-03 — paced AI-vs-AI demo (main session)

- **Menu "Watch two computers"** (`MenuButton::Watch`): deals a two-player
  board to camps {0,3}, marks every seat as the computer's, and sends the
  session straight to `InGame`. Spectator marking in `apply_seats` blocks board
  input the same way networked spectating does.
- **Paced driver** (`checkers-bevy/src/ai.rs`): `AiPace` throttles one visible
  action (move, commit, or hop) per second and stages jumps through the rules'
  own `JumpTurn` so the preview shows the piece mid-flight — a human can follow
  the race, and every committed move is one the rules offer.
- **`choose_move_route_for`** on `Ai`: returns the chosen move plus a landings-only
  hop route (via `rules::jump_routes`, origin stripped) so the driver can animate
  a jump hop by hop. Covered by `checkers-ai/tests/route.rs` (the two-rung chain).
- **Headless demo test** (`checkers-bevy/tests/ai_demo.rs`): injected clock; asserts
  1-second spacing, legal committed moves, per-ply audit, and termination — the
  two-engine race resolves with a winner (90 plies).
- **Stall backstop**: `AiPace` carries a progress-stall detector (leading seat's
  sum-of-distance window) and a hard `MAX_MOVES` ceiling; either logs an honest
  `# game abandoned` line, so a genuinely unresolvable game never runs forever.
- **Engine eval experiment, reverted**: an attempted evacuation tax (per-piece
  own-start-camp penalty ×80) and a wrong-camp dead-end penalty both *regressed*
  the engine — `play.rs` self-play stopped finishing. Reverted to the original
  `worst*12` eval; the existing anti-shuffle is what keeps a real race on track.
  The lesson: the demo's termination safety belongs in the paced driver, not the
  eval.

### Session record
- All gates green: fmt, clippy `--workspace --all-targets`, `test --workspace`
  (25 test binaries, 0 failures), `doc --workspace --no-deps`, spec-gen
  (`--check`, `--check-registry`), wasm build for `checkers-bevy`.

## 2026-09-03 — step staging, menu background, rebase audit (main session)

- **Steps are staged** (`57ad03d`): a single step lands in
  `Selection::Pend { mv, preview }` and waits for Enter, exactly like a jump
  chain; `preview` shows the piece at its destination before the commit.
  Controls go inert on another player's turn (`Session::may_act` gates the
  buttons, clicks, and the staged-turn keys; `R`/`V`/`T`/`Escape` stay live).
- **Menu background**: a live two-bot race behind the main menu
  (`menu_bg`), drawing into the startup camera with its own
  `MenuDemo { session, ai, pace }`, rebuilt only on landed moves, despawned
  on exit. `Session::deal_two` and a shared `board_view::player_colour` keep
  it identical to the watched demo; the button is now "Watch 2 Bots"; the
  menu sits on a translucent card for legibility.
- **Rebase-conflict audit**: the parallel session's two `pull --rebase
  --autostash` runs left both autostashes unpopped. Audit found nothing lost:
  `stash@{0}` reverse-applies cleanly onto HEAD; `stash@{1}`'s features
  landed in evolved form (`for_players`/`compose`, `apply_seats`,
  `turn_order_skips_vacant_seats` supersedes its UI-level test). Both stashes
  dropped after verification.
- **Roadmap**: remaining work planned and written to `docs/ROADMAP.md`
  (decisions: resign is button-only, `.cchkrs` records, AI strength 1–5).

## 2026-09-04 — the roadmap, end to end (main session)

- **Phases 1–3**: README honesty (steps are staged; every key documented);
  the round-duration readout that `57ad03d`'s message had promised but the
  rebase had eaten, restored; resignation as a core `Outcome` with a
  button-only control; engine strength 1–5 with level 3 pinned to the old
  default.
- **Track A, closed by surprise**: the claim inventory found chapters 6–15
  already fully formalised (28 of 43 laws); the README paragraph advertising
  the gap was itself stale. `docs/claim-inventory.md` records the mapping
  and the two declined candidates (turn-kind purity, win terminality).
- **Phase 4**: `.cchkrs` records — the position is derived, not stored;
  resume replays every move through `WireMove::resolve` with the law audit
  per move, so a forged record is refused rather than resumed. Native file
  dialog, web localStorage. The replay viewer walks a record: arrows step,
  Space autoplays, the on-screen session is rebuilt at the cursor via
  `resumed_prefix`, and while it is up the play systems stand down.
- **Phase 5**: lobby Add engine seats a host-owned engine; it takes a camp
  in join order, reads as ready, and only the sequencing authority's engine
  drives it — moves flow as ordinary sequenced moves, so a guest cannot
  double-drive. No host migration, by the standing rule.
- **Phase 6**: own-move animation declined with reasoning — hotseat already
  animates own moves; in solo they would delay the player's own feedback.
- **Phase 7**: one-finger orbit, pinch zoom, and tap-as-click (winit
  prevents emulated mouse events on the web canvas). The on-glass feel
  spike stays manual.
- **Phase 8**: sound via Bevy's built-in `Pitch` — the file/script question
  dissolved; `M` mutes; the first click is the browser's gesture.
- Two mid-flight collisions with the parallel session's own feature work
  (a replay-animation module landing in the same `Session`): waited out the
  edit-loop churn, merged coherently, and committed entangled slices
  together per the standing policy. One of their fresh tests asserted the
  opposite of `CC-JUMP-NO-CAPTURE`; fixed to assert the law.

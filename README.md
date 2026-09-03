# Chinese Checkers — specification and implementation

A specification of six-player (star-shaped) Chinese Checkers, and a Rust
implementation that is mechanically checked against it.

## The traceability chain

The problem this layout solves: a `// §16` comment is just a string. Nothing
detects it when a section is renumbered, a rule changes, or a claim is dropped.
So every normative claim is a **Rust type** instead:

```rust
/// $\forall i \in P:\ |\{v \in V : s(v) = i\}| = 10$
pub struct PieceConservation;

impl Law for PieceConservation {
    const ID: &'static str = "CC-POS-PIECES";   // stable anchor
    const STATEMENT: &'static str = r"\forall i \in P: \ldots"; // the mathematics
    const CHAPTER: Chapter = Chapter::Players;  // provenance, as a type
    const SUMMARY: &'static str = "Every player owns exactly ten pieces in every position.";
    const NOTE: &'static str = "Every player always owns exactly ten pieces.";
    const EVIDENCE: Evidence = Evidence::Property; // how it is established
    type Subject = Position;                    // what the ∀ ranges over
    fn holds(p: &Position) -> Result<(), String> { /* the check */ }
}
register_law!(PieceConservation, PIECE_CONSERVATION);
```

Identity, mathematics, provenance, gloss, plain-language note, domain, and
check live in one block and cannot drift. The note is the claim restated for
a reader who skips the mathematics; the guard tests require it to be a
sentence without a single backslash.
Registration happens at **link time** (`linkme` distributed slice), so:

- `cargo test` runs every law without naming any — you cannot declare a law and
  forget to check it;
- `checkers-spec-gen` documents every law from the same registry — a law cannot
  be documented without being checked, or checked without being documented.

`Subject` is what makes this more than decoration: it names the domain the
quantifier ranges over, so the same law can be exhaustively enumerated, property
tested, or proven.

### Levels of evidence

| Level | Meaning |
|---|---|
| `proof (Kani)` | Proven for the whole domain by bounded model checking. |
| `exhaustive` | Checked over a finite domain in ordinary Rust. |
| `property test` | Checked over inputs from a `proptest` strategy. |
| `example` | Fixed examples only — weakest. |

**What this does not give you.** `holds` is a runtime predicate; the type system
does not verify that `STATEMENT` means what `holds` computes. That gap is
irreducible without a proof assistant. The value is that a reviewer sees both in
one place, and that the strongest claims are additionally Kani-proven.

## Layout

| Crate | Role |
|---|---|
| `checkers-core` | Engine-free rules. Laws, geometry, Kani proofs. |
| `checkers-model` | Naive reference implementation, played in lockstep against `checkers-core` by its `tests/differential.rs`. |
| `checkers-spec-gen` | Generates `specs/` from the law registry. |
| `checkers-bevy` | Playable front-end. Renders and takes input; holds no rules. |
| `specs/` | **Generated.** Do not edit; regenerate instead. |
| `docs/superseded/` | The original hand-written prose. Not authoritative. |

The two hand-written specification documents have been removed. Their prose now
lives in `checkers-core/src/spec.rs` as [`Chapter`] text, and the generated
`specs/specification.md` supersedes them.

`docs/superseded/` retains the originals only as a source for prose not yet
transcribed. All fifteen chapters are formalised: every normative claim is
backed by a law, and the claims that are not — deliberately — are recorded
with their reasons in `docs/claim-inventory.md`.

### Reading order

Chapters are a `Chapter` enum, not section numbers, so a law cannot cite a section
that does not exist and there is no numbering to fall out of date. The enum's
declaration order *is* the reading order, which is why the specification is
generated rather than read out of rustdoc: rustdoc sorts items alphabetically and
offers no stable way to override that (`--sort-modules-by-appearance` is
nightly-only and orders modules, not items).

Code cites **law IDs**, never section numbers.

## Commands

```sh
cargo test                                          # all laws + tests
cargo run -p checkers-bevy                          # play
cargo doc --no-deps --open                           # rustdoc with rendered math
cargo run -p checkers-spec-gen -- specs/specification.md          # regenerate
cargo run -p checkers-spec-gen -- --check specs/specification.md  # CI staleness gate
```

### Rendered math in rustdoc

rustdoc has **no native math support** — it strips `\(...\)` and passes `$$`
through as literal text. KaTeX is injected via a rustdoc flag in
`.cargo/config.toml`:

```toml
[build]
rustdocflags = ["--html-in-header", "assets/katex-header.html"]
```

This must be a *flag*: `#![doc(html_in_header = ...)]` as a crate attribute is
rejected by cargo as an unknown attribute. For docs.rs, declare it under
`[package.metadata.docs.rs]` as well, since docs.rs ignores `.cargo/config.toml`.

The path is relative, and the flag applies to **every crate rustdoc
documents** — so documenting a dependency, whose sources have no `assets/`,
fails with `error reading assets/katex-header.html`. Always pass
`--no-deps` (CI does).

### Running the proofs

**Kani does not build on Windows** — it hard-depends on `std::os::unix`. Proofs
therefore run under Linux, WSL, or CI, while `cargo test`, `cargo doc`, and the
eventual Bevy app all work natively on Windows.

```sh
cargo install kani-verifier && kani setup   # once, on Linux/WSL
./scripts/verify-proofs.sh
```

From a Windows shell:

```sh
MSYS_NO_PATHCONV=1 wsl.exe -d Debian bash /mnt/c/workspace/sources/amlah-spec/scripts/verify-proofs.sh
```

Current status: **11 proof harnesses, all verifying.**

### Writing provable code

Bounded model checking constrains how the geometry is written: a symbolic loop
bound makes Kani diverge. `rotate_n` is therefore a loop-free `match` over
`n % 6` rather than an idiomatic `fold` — an earlier `fold` version sent Kani
unwinding past 6900 iterations. Keep functions intended for proof branch-bounded
and arithmetic.

## Playing

`cargo run -p checkers-bevy`

The main menu deals a game: play solo or against the computer, hand a seat to
another person at the keyboard (hotseat), join a networked lobby, or watch two
engines race ("Watch 2 Bots" — the menu itself sits over a live one). The
hotseat panel picks the engine's strength from 1 (quick) to 5 (strong); the
choice is applied when the game is dealt. When it is not your turn, the turn
controls are inert; a spectator can watch but never click.

Every turn is **staged** — steps and jumps alike. Selecting a piece highlights
the destinations reachable in **one** hop; taking one moves the piece in the
view, keeps it selected, and for a jump reveals the next hop. Nothing is
committed until you confirm, so any chain can be abandoned.

| Input | Effect |
|---|---|
| Click own piece | Select it |
| Click a highlighted hole | Stage a step, or take one hop of a jump |
| Enter / Confirm button | Commit the staged turn |
| Backspace / Cancel button | Abandon the staged turn |
| Resign button | Concede the round (local modes; not over the network yet) |
| Save / Open buttons | Write the round to a `.cchkrs` record, or resume one |
| U | Undo the last staged hop |
| Escape | Clear the selection |
| A | Let the computer play the current seat once |
| T | Toggle the status panel |
| V | Cycle board style: classic (2D) / amlah (3D) |
| R | Restart with a fresh two-player deal |
| Right-drag, wheel (amlah) | Orbit and zoom the 3D camera |

Confirming before the piece has actually moved is refused. That is not a corner
case to be tidied away: a piece can hop out over a blocker and straight back, so
a turn can have taken two hops and still be at its origin — which chapter 9
treats as not moving.

A finished game shows a summary card: who won (or that every player is
blocked), each seated player's moves, how many were jumps, and how long the
round lasted.

The front-end holds no rules. Destinations come from `checkers-core` and moves
are *found* rather than constructed, so there is no code path to an illegal
position. It checks the full law registry once at startup and the position
invariants after every committed turn.

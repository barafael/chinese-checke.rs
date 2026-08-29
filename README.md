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
    const ID: &'static str = "CC-INV-PIECES";   // stable anchor
    const STATEMENT: &'static str = r"...";     // the mathematics
    const SECTION: &'static str = "Impl §28";   // provenance
    const EVIDENCE: Evidence = Evidence::Proof; // how it is established
    type Subject = Position;                    // what the ∀ ranges over
    fn holds(p: &Position) -> Result<(), String> { /* the check */ }
}
register_law!(PieceConservation, PIECE_CONSERVATION);
```

Statement, provenance, domain, and check live in one block and cannot drift.
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
| `checkers-model` | Naive reference implementation, for differential testing. |
| `checkers-spec-gen` | Generates `specs/` from the law registry. |
| `specs/` | **Generated.** Do not edit; regenerate instead. |
| `docs/superseded/` | The original hand-written prose. Not authoritative. |

The two hand-written specification documents have been removed. Their prose now
lives in `checkers-core/src/spec.rs` as [`Chapter`] text, and the generated
`specs/specification.md` supersedes them.

`docs/superseded/` retains the originals only as a source for prose not yet
transcribed. The geometry chapters (1–5) are done; what remains to verify against
the originals is the detail of chapters 6–15, whose claims are currently stated in
prose but **not** yet formalised as laws. The generated document lists exactly
which those are, under "Coverage".

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
cargo doc --open                                    # rustdoc with rendered math
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

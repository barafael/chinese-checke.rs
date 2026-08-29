# Chinese Checkers — specification

<!-- GENERATED FILE. Do not edit.
     Source: checkers-core/src/spec.rs and checkers-core/src/laws/
     Regenerate: cargo run -p checkers-spec-gen -- specs/specification.md -->

This document is generated from `checkers-core`. Do not edit it; edit the chapter prose in `checkers-core/src/spec.rs` or the law impls in `checkers-core/src/laws/`, then regenerate.

Each numbered chapter states the rules in prose and mathematics. The **laws** listed under a chapter are the machine-checked formalisation of its claims: every law is a Rust type whose statement, provenance, and executable check live in one place, and which is registered at link time so it cannot be documented without being verified.

## Contents

1. [Coordinates and directions](#coordinates) — 1 law
2. [The central hexagon](#hexagon) — 1 law
3. [The six camps](#camps) — 4 laws
4. [Rotation and opposite camps](#rotation) — 5 laws
5. [The complete board](#board) — 4 laws
6. [Players, pieces, and the initial position](#players) — 4 laws
7. [Adjacent moves](#steps) — 2 laws
8. [Jumps](#jumps) — 4 laws
9. [Jump sequences and reachability](#jump-sequences) — 5 laws
10. [Move representation and generation](#move-generation) — 4 laws
11. [Applying a move](#applying) — 1 law
12. [Turn order, passing, and termination](#turns) — 3 laws
13. [The winning condition](#winning) — 1 law
14. [Position invariants](#invariants) — 1 law
15. [Rule variants](#variants) — 1 law

## Coverage

| Evidence | Laws |
|---|---|
| exhaustive | 15 |
| proof (Kani) | 11 |
| property test | 15 |
| **total** | **41** |

Each law records how strongly it is established:

| Evidence | Meaning |
|---|---|
| proof (Kani) | Proven for the whole domain by bounded model checking. |
| exhaustive | Checked over a finite domain by enumeration. |
| property test | Checked over inputs from a generated strategy. |
| example | Checked against fixed examples only. |

`proof (Kani)` laws additionally re-check themselves in ordinary Rust, so `cargo test` exercises them on every platform; the proofs themselves need Linux or WSL, since Kani does not build on Windows.

---

## 1. Coordinates and directions <a id="coordinates"></a>

Every playable hole is identified by an axial hex coordinate $(q,r) \in \mathbb{Z}^2$, with the third cube coordinate implicit as $s = -q-r$.

Six directions connect adjacent holes:

$$D = \{(1,0),\ (1,-1),\ (0,-1),\ (-1,0),\ (-1,1),\ (0,1)\}$$

Two holes $u, v$ are adjacent exactly when $v - u \in D$. The listing above is in rotational order, so consecutive directions are $60^\circ$ apart. Only the set matters to the rules; the order is fixed so that direction indices are stable.

In the implementation, directions are an enumeration rather than a collection of vectors, which makes "there are exactly six directions" a structural fact: no seventh direction is representable.

#### Laws

##### `CC-DIR-INVOLUTION`

Directions come in opposite pairs, so adjacency is symmetric.

$$
\forall d \in D:\ -d \in D \ \land\ -(-d) = d
$$

*Evidence: proof (Kani)*

## 2. The central hexagon <a id="hexagon"></a>

The board's centre is a hexagon of radius four:

$$H_4 = \{(q,r) \in \mathbb{Z}^2 : |q| \le 4,\ |r| \le 4,\ |q+r| \le 4\}$$

It contains $1 + 6(1+2+3+4) = 61$ holes.

All three constraints are required. Dropping the bounds on $q$ and $r$ leaves $|q+r| \le 4$, which describes an unbounded diagonal strip rather than a hexagon.

#### Laws

##### `CC-GEO-NONVACUOUS`

The hexagon, each camp, and the board are inhabited with their stated sizes.

$$
|H_4| = 61 \land \forall i:\ |C_i| = 10 \land |V| = 121
$$

*Evidence: proof (Kani)*

## 3. The six camps <a id="camps"></a>

Six triangular camps of ten holes each surround the hexagon. Camp $C_0$ is seated flush against the hexagon's $q = 4$ edge:

$$C_0 = \{(q,r) \in \mathbb{Z}^2 : 5 \le q \le 8,\ -4 \le r \le -(q-4)\}$$

Its columns hold $4, 3, 2, 1$ holes, decreasing outward to a single apex at $(8,-4)$. The triangle therefore points **away** from the centre, and its four-hole base lies against the hexagon.

The orientation is the whole content of this chapter, and it is easy to get wrong. The inward-pointing variant

$$C_0^{\text{bad}} = \{(q,r) : 5 \le q \le 8,\ -q+5 \le r \le 0\}$$

also has ten holes per camp and also yields a 121-hole board, so every cardinality check passes. But it meets the hexagon at the single hole $(5,0)$ instead of along a four-hole edge, leaving the six camps dangling from the hexagon's corners. It is not a six-pointed star.

Cardinality alone cannot detect this. The distinguishing property is contact with the hexagon: a correct camp has four holes adjacent to $H_4$, contributing eight camp-to-hexagon adjacent pairs, whereas the degenerate one has a single contact hole and a single adjacent pair.

#### Laws

##### `CC-GEO-BASE-CAMP`

Camp 0 occupies columns q=5..8 with 4,3,2,1 holes, apex outward at (8,-4).

$$
C_0 = \{(q,r) : 5 \le q \le 8,\ -4 \le r \le -(q-4)\},\quad \text{columns } 4,3,2,1
$$

*Evidence: exhaustive*

##### `CC-GEO-CONTACT`

Each camp's four-hole base sits flush against a hexagon edge, giving eight contact pairs.

$$
\forall i:\ \left|\{(x,y) \in C_i \times H_4 : y - x \in D\}\right| = 8
$$

*Evidence: exhaustive*

##### `CC-GEO-INWARD-BAD`

The inward-pointing camp variant touches the hexagon at one hole, so it is not a star.

$$
\left|\{x \in C_0^{\text{bad}} : \exists d \in D,\ x + d \in H_4\}\right| = 1
$$

*Evidence: proof (Kani)*

##### `CC-GEO-INWARD-CONTACT`

The inward camp has one hexagon contact pair, against eight for a correct camp.

$$
\left|\{(x,y) \in C_0^{\text{bad}} \times H_4 : y - x \in D\}\right| = 1
$$

*Evidence: exhaustive*

## 4. Rotation and opposite camps <a id="rotation"></a>

The remaining camps are rotations of $C_0$. A $60^\circ$ rotation in axial coordinates is

$$R(q,r) = (-r,\ q+r)$$

and $C_i = R^i(C_0)$ for $i = 0, \ldots, 5$.

$R$ has order six, and $R^3 = -\mathrm{id}$. The second fact is the important one: it means the camp three positions away is the point reflection of the original,

$$C_{(i+3) \bmod 6} = -C_i = \{-x : x \in C_i\}$$

which is why a player's target is camp $i+3$ — it is geometrically opposite, directly across the centre.

Whether $R$ reads as clockwise or counter-clockwise depends on the rendering convention and is not fixed here. Nothing in the rules depends on the choice, only on $R$ having order six and the camps being indexed consistently.

Note that $R$ sends $(1,0) \mapsto (0,1)$, stepping *backwards* through the direction order of chapter 1. That is harmless but worth knowing when comparing indices.

#### Laws

##### `CC-GEO-OPPOSITE`

A player's target camp is the point reflection of their start.

$$
C_{(i+3) \bmod 6} = -C_i = \{-x : x \in C_i\}
$$

*Evidence: proof (Kani)*

##### `CC-GEO-ROT-EXACT`

The rotation has order exactly six: no smaller power is the identity.

$$
\text{ord}(R) = 6:\quad R^6 = \mathrm{id} \land \forall k \in \{1..5\}:\ R^k \neq \mathrm{id}
$$

*Evidence: proof (Kani)*

##### `CC-GEO-ROT-NEG`

Three rotations equal point reflection, which is why camp i+3 is opposite camp i.

$$
\forall x \in \mathbb{Z}^2:\ R^3(x) = -x
$$

*Evidence: proof (Kani)*

##### `CC-GEO-ROT-ORDER`

The 60-degree rotation has order six.

$$
\forall x \in \mathbb{Z}^2:\ R^6(x) = x
$$

*Evidence: proof (Kani)*

##### `CC-GEO-ROT-STEP`

Rotation maps each direction to its neighbour in the cycle.

$$
\forall k:\ R(d_k) = d_{k-1 \bmod 6}
$$

*Evidence: proof (Kani)*

## 5. The complete board <a id="board"></a>

The board is the disjoint union of the hexagon and the six camps:

$$V = H_4 \mathbin{\dot\cup} C_0 \mathbin{\dot\cup} \cdots \mathbin{\dot\cup} C_5,\qquad |V| = 61 + 6 \cdot 10 = 121$$

A correct construction satisfies more than its cardinality. Each camp must meet the hexagon in exactly eight adjacent pairs; the board graph must be connected; and the whole board must be centrally symmetric, $V = -V$.

A useful end-to-end check is that all six players have the same number of legal moves in the initial position — fourteen on the standard board. This follows from six-fold symmetry, so it fails loudly if a camp is misplaced.

Rendering the construction is the fastest way to catch a mistake:

```text
               5
              5 5
             5 5 5
            5 5 5 5
 4 4 4 4 . . . . . 0 0 0 0
  4 4 4 . . . . . . 0 0 0
   4 4 . . . . . . . 0 0
    4 . . . . . . . . 0
     . . . . . . . . .
    3 . . . . . . . . 1
   3 3 . . . . . . . 1 1
  3 3 3 . . . . . . 1 1 1
 3 3 3 3 . . . . . 1 1 1 1
          2 2 2 2
           2 2 2
            2 2
             2
```

Each row $r$ is drawn at horizontal offset $2q + r$. Opposite camps sit diametrically across the centre, as chapter 4 requires.

#### Laws

##### `CC-GEO-CAMP-OF`

Camp lookup agrees with camp membership on every hole.

$$
\forall x \in V:\ \mathrm{camp}(x) = i \iff x \in C_i,\quad \mathrm{camp}(x) = \bot \iff x \in H_4
$$

*Evidence: exhaustive*

##### `CC-GEO-CARDINALITY`

Hexagon of 61 holes plus six camps of 10 gives 121 holes.

$$
|V| = 61 + 6 \cdot 10 = 121,\quad |H_4| = 61,\quad |C_i| = 10
$$

*Evidence: exhaustive*

##### `CC-GEO-DISJOINT`

The hexagon and the six camps are pairwise disjoint, so every hole is covered once.

$$
V = H_4 \mathbin{\dot\cup} C_0 \mathbin{\dot\cup} \cdots \mathbin{\dot\cup} C_5
$$

*Evidence: proof (Kani)*

##### `CC-GEO-SYMMETRY`

The board is symmetric under point reflection through centre.

$$
\forall x:\ x \in V \iff -x \in V
$$

*Evidence: proof (Kani)*

## 6. Players, pieces, and the initial position <a id="players"></a>

Six players each own ten indistinguishable pieces. A position is an occupancy function

$$s : V \rightarrow P \cup \{\varnothing\},\qquad P = \{0,\ldots,5\}$$

with every hole holding at most one piece. Player $i$ starts with all ten pieces in camp $C_i$ and every other hole empty, so a valid position always has 60 occupied and 61 empty holes.

Player $i$'s target is the opposite camp, $O_i = C_{(i+3) \bmod 6}$.

#### Laws

##### `CC-POS-INITIAL`

Initially each player fills their own camp and the hexagon is empty.

$$
s_0(v) = i \iff v \in C_i,\qquad s_0(v) = \varnothing \iff v \in H_4
$$

*Evidence: exhaustive*

##### `CC-POS-OCCUPANCY`

Sixty holes are occupied and sixty-one empty, totalling 121.

$$
\left|\{v : s(v) \neq \varnothing\}\right| = 60 \ \land\ \left|\{v : s(v) = \varnothing\}\right| = 61
$$

*Evidence: property test*

##### `CC-POS-PIECES`

Every player owns exactly ten pieces in every position.

$$
\forall i \in P:\ \left|\{v \in V : s(v) = i\}\right| = 10
$$

*Evidence: property test*

##### `CC-POS-TARGET`

A player's target is the opposite camp, distinct from their start, and the pairing is mutual.

$$
O_i = C_{(i+3) \bmod 6},\qquad O_{O_i} = C_i,\qquad O_i \cap C_i = \varnothing
$$

*Evidence: exhaustive*

## 7. Adjacent moves <a id="steps"></a>

A turn moves exactly one piece belonging to the active player, and is either one adjacent step or a sequence of jumps — never a mixture.

A piece at $x$ may step to $y$ exactly when

$$s(x) = i \ \land\ y - x \in D \ \land\ s(y) = \varnothing$$

that is, the destination is adjacent and empty. The piece vacates $x$ and occupies $y$; nothing else changes.

#### Laws

##### `CC-STEP-DISPLACE`

A step moves to an adjacent hole, never further and never onto a piece.

$$
y - x \in D \implies \mathrm{dist}(x,y) = 1
$$

*Evidence: property test*

##### `CC-STEP-LEGAL`

Generated steps are exactly the adjacent, on-board, empty destinations.

$$
\mathrm{step}(x,y) \iff s(x) = i \ \land\ y - x \in D \ \land\ y \in V \ \land\ s(y) = \varnothing
$$

*Evidence: property test*

## 8. Jumps <a id="jumps"></a>

A piece at $x$ may jump in direction $d$ to $x + 2d$ exactly when the intervening hole is occupied and the landing hole is an empty board hole:

$$x+d \in V \ \land\ s(x+d) \neq \varnothing \ \land\ x+2d \in V \ \land\ s(x+2d) = \varnothing$$

The jumped piece is **never** captured or removed, and may belong to any player — legality depends only on the hole being occupied, not on who occupies it. The only occupancy changes are that $x$ becomes empty and $x+2d$ becomes the mover's.

Since $d \neq 0$, a jump displaces by $2d \neq 0$ and so can never land on its own origin. The jumped hole is exactly the midpoint of the hop.

#### Laws

##### `CC-JUMP-ANY-OWNER`

A piece may be jumped regardless of which player owns it.

$$
\mathrm{jump}(x,d) \text{ depends on } s(x+d) \neq \varnothing, \text{ not on } s(x+d)
$$

*Evidence: exhaustive*

##### `CC-JUMP-DISPLACEMENT`

A hop moves by twice a direction; the jumped hole is exactly the midpoint.

$$
\forall x, d \in D:\ x + 2d \neq x \ \land\ 2(x+d) = x + (x+2d)
$$

*Evidence: proof (Kani)*

##### `CC-JUMP-LEGAL`

A jump needs an occupied hole to cross and an empty hole to land on.

$$
\mathrm{jump}(x,d) \iff x+d \in V \ \land\ s(x+d) \neq \varnothing \ \land\ x+2d \in V \ \land\ s(x+2d) = \varnothing
$$

*Evidence: property test*

##### `CC-JUMP-NO-CAPTURE`

Jumping leaves the crossed piece in place and removes nothing.

$$
s'(x+d) = s(x+d),\qquad \left|\{v : s'(v) \neq \varnothing\}\right| = \left|\{v : s(v) \neq \varnothing\}\right|
$$

*Evidence: property test*

## 9. Jump sequences and reachability <a id="jump-sequences"></a>

A turn may chain arbitrarily many jumps, all performed by the **same** piece, changing direction freely between hops. The player may stop after any jump; continuing is optional.

Legality is evaluated against the position produced by the preceding jumps. That statement is true but routinely over-read, so it is worth being precise about what does and does not depend on the evolving position.

Because a turn moves only one piece and jumps never capture, the occupancy of every *other* hole is fixed for the whole turn. Writing $\Omega$ for the occupied holes excluding the moving piece, a jump from the piece's current hole $x$ is legal exactly when

$$x+d \in \Omega \ \land\ x+2d \in V \setminus \Omega$$

Only $x$, $d$, and the fixed set $\Omega$ appear. Within a single turn, therefore, **occupancy is a function of the moving piece's position**, and the available jumps depend on that position alone. The moving piece can never block itself, since it is excluded from $\Omega$.

The consequence is practical: the set of reachable destinations is the forward closure of a directed graph fixed once per turn, so a breadth-first search over **positions**, with a single visited set, computes it exactly and always terminates.

It is tempting to conclude instead that the search must be keyed on the pair (position, board state), on the grounds that the board changes after each hop. That is a mistake. Within a turn the state is determined by the position, so such a key can never distinguish two visits that the position alone would not — while making the search appear to need unbounded state. A search keyed that way does not terminate.

Jump sequences genuinely may revisit holes, including the starting hole: with a blocker adjacent, a piece can hop out and straight back. The space of jump *paths* is therefore infinite, and any procedure enumerating paths needs an explicit guard — forbidding repeats within the current path is the natural choice. The space of *destinations* is finite regardless, which is all move generation requires. A turn is conventionally required to end somewhere other than where it began, since ending at the origin is indistinguishable from not moving.

#### Laws

##### `CC-JUMP-CLOSURE`

Breadth-first search over positions yields exactly the routes' destinations.

$$
\{y : x \leadsto_s y\} = \{\mathrm{last}(\pi) : \pi \text{ a simple jump route from } x\}
$$

*Evidence: property test*

##### `CC-JUMP-OMEGA`

The other pieces never move during a turn, so available jumps depend only on position.

$$
\Omega = \{v : s(v) \neq \varnothing\} \setminus \{x_0\} \text{ is invariant during a turn}
$$

*Evidence: property test*

##### `CC-JUMP-REVISIT`

A piece can jump out and back, so unguarded route enumeration does not terminate.

$$
\exists s, x, d:\ x \rightarrow x+2d \rightarrow x, \text{ so the route space is infinite}
$$

*Evidence: exhaustive*

##### `CC-TURN-HOP-CLOSURE`

Chaining single hops reaches exactly the destinations the closure allows.

$$
\{y : x \leadsto_s y\} = \text{closure of single hops from } x
$$

*Evidence: property test*

##### `CC-TURN-HOP-ONE`

Every single-hop destination lies exactly two holes away in one direction.

$$
\forall y \in H(s,x):\ \exists d \in D:\ y = x + 2d
$$

*Evidence: property test*

## 10. Move representation and generation <a id="move-generation"></a>

A move is identified by its kind, origin, and destination — not by the route taken. Distinct jump routes to the same hole produce the same resulting position, so they are the same move; counting them separately inflates move counts and any search built on them.

A route may still be recorded for animation or notation, but it must be excluded from equality and hashing.

The complete legal move set for player $i$ is the adjacent steps from each of their pieces, together with one jump move per reachable destination.

#### Laws

##### `CC-MOVE-DEDUP`

Move generation yields one move per reachable destination, with no duplicates.

$$
M_{\text{jump}}(s,i) = \{(x,y) : s(x) = i,\ y \in J^{*}(s,i,x)\}, \text{ without repetition}
$$

*Evidence: property test*

##### `CC-MOVE-IDENTITY`

Two routes to the same hole are the same move, so routes are not part of identity.

$$
m_1 = m_2 \iff (\mathrm{kind}, x, y)_1 = (\mathrm{kind}, x, y)_2
$$

*Evidence: exhaustive*

##### `CC-MOVE-ONBOARD`

Every generated move starts on one of the player's pieces and ends on the board.

$$
\forall m \in M(s,i):\ x, y \in V \ \land\ s(x) = i
$$

*Evidence: property test*

##### `CC-TURN-STAGED-LEGAL`

Any sequence of legal single hops commits to a move the rules already allow.

$$
\text{hops } h_1\ldots h_k \text{ legal} \implies (x, h_k) \in M(s,i)
$$

*Evidence: property test*

## 11. Applying a move <a id="applying"></a>

Applying a move vacates the origin and occupies the destination. For a jump sequence, no intermediate hole is modified and nothing is captured, so

$$s'(x) = \varnothing,\qquad s'(y) = i,\qquad s'(z) = s(z)\ \ \forall z \notin \{x,y\}$$

Replaying a route hole-by-hole and applying this net effect therefore agree, which is what justifies identifying moves by destination rather than by route.

#### Laws

##### `CC-APPLY-NET`

Replaying a route hole-by-hole gives the same position as applying the net effect.

$$
s'(x) = \varnothing,\quad s'(y) = i,\quad s'(z) = s(z)\ \ \forall z \notin \{x,y\}
$$

*Evidence: property test*

## 12. Turn order, passing, and termination <a id="turns"></a>

Turns proceed cyclically through the six players.

The rules do **not** guarantee the active player has a legal move. The situation is reachable: if a player's ten pieces fill a camp, opponents can occupy that camp's frontier holes and the holes beyond them, leaving no step and no jump available. Such a player still holds all ten pieces, so this is neither a win nor a loss.

This specification resolves it by **passing**: a player with no legal move forfeits the turn and play continues. If all six players pass in succession the position cannot change, and the game is a draw.

One consequence is that the active player is not simply the turn number modulo six, since passing advances the player without consuming a turn in that sense. Implementations should track the active player as explicit state rather than deriving it.

Rule sets differ here. Passing is the least intrusive convention; others forbid the blocking configuration outright, or oblige the blocking player to move aside.

#### Laws

##### `CC-TURN-BLOCKED`

A player can have no legal move yet all ten pieces, which is neither a win nor a loss.

$$
\exists s, i:\ T_i(s) = \varnothing \ \land\ \left|\{v : s(v) = i\}\right| = 10 \ \land\ \neg\mathrm{Won}(s,i)
$$

*Evidence: exhaustive*

##### `CC-TURN-CYCLE`

Turn order cycles through all six players and returns.

$$
\mathrm{next}(i) = (i+1) \bmod 6,\qquad \mathrm{next}^6 = \mathrm{id}
$$

*Evidence: exhaustive*

##### `CC-TURN-PASS`

A player with no move passes; six consecutive passes end the game in a draw.

$$
T_i(s) = \varnothing \implies \text{pass};\qquad \text{six passes} \implies \text{draw}
$$

*Evidence: exhaustive*

## 13. The winning condition <a id="winning"></a>

Player $i$ wins by occupying every hole of the opposite camp:

$$\forall x \in C_{(i+3) \bmod 6}:\ s(x) = i$$

Since the target camp has ten holes and the player has exactly ten pieces, this is equivalent to all of their pieces having arrived. The game ends at the first position satisfying this for some player.

#### Laws

##### `CC-WIN-CONDITION`

A player wins exactly when every hole of the opposite camp holds one of their pieces.

$$
\mathrm{Won}(s,i) \iff \forall v \in C_{(i+3) \bmod 6}:\ s(v) = i
$$

*Evidence: exhaustive*

## 14. Position invariants <a id="invariants"></a>

Every completed move preserves the following. Each player owns exactly ten pieces; exactly 60 holes are occupied and 61 empty; every hole holds at most one piece. No move creates, destroys, or transfers ownership of a piece — jumping is not capture.

#### Laws

##### `CC-INV-PRESERVED`

Every legal move preserves each player's piece count and total occupancy.

$$
(s, s') \in T_i \implies \forall j:\ \left|\{v : s'(v) = j\}\right| = \left|\{v : s(v) = j\}\right|
$$

*Evidence: property test*

## 15. Rule variants <a id="variants"></a>

Two points are deliberately left open, and an implementation must choose explicitly rather than letting the choice emerge from its geometry code.

**Camp restrictions.** This specification uses the unrestricted convention: camp membership imposes no additional movement constraint, so a piece may enter, leave, or move within any camp subject only to the ordinary rules. Some rule sets restrict occupation of an opponent's camp. Such a restriction belongs in a separate legality predicate, not embedded in the movement rules.

**Blocked players.** See chapter 12. This specification passes; other conventions exist.

#### Laws

##### `CC-VAR-CAMP-FREE`

Under the unrestricted convention a piece may enter, leave, or cross any camp.

$$
\mathrm{CampLegal}(s, i, m) \equiv \text{true}
$$

*Evidence: exhaustive*


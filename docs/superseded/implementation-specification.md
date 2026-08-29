# Chinese Checkers — Implementation Specification

## 1. Coordinate system

Represent every playable hole by an **axial hex-grid coordinate**

$$
q,r\in\mathbb Z.
$$

The six neighboring directions are

$$
D=
\{
(1,0),
(1,-1),
(0,-1),
(-1,0),
(-1,1),
(0,1)
\}.
$$

For convenience, name them

$$
d_0=(1,0),\quad
d_1=(1,-1),\quad
d_2=(0,-1),
$$

$$
d_3=(-1,0),\quad
d_4=(-1,1),\quad
d_5=(0,1).
$$

For coordinates $x=(q,r)$ and $y=(q',r')$,

$$
y-x=(q'-q,r'-r).
$$

They are adjacent iff

$$
y-x\in D.
$$

### Pseudocode

```text
# Axial coordinates:
#
# x = (q, r), q,r ∈ Z
#
# D = {
#     ( 1,  0),
#     ( 1, -1),
#     ( 0, -1),
#     (-1,  0),
#     (-1,  1),
#     ( 0,  1)
# }
#
# Adjacent(x, y) ⇔ y - x ∈ D
```

---

## 2. The 61-hole central hexagon

Define the radius-4 hexagon

$$
H_4=
\left\{
(q,r)\in\mathbb Z^2:
|q|\le4,\;
|r|\le4,\;
|q+r|\le4
\right\}.
$$

This contains exactly

$$
1+6(1+2+3+4)=61
$$

holes.

Thus

$$
|H_4|=61.
$$

### Pseudocode

```text
function CentralHex():
    H = empty set

    for q from -4 to 4:
        for r from -4 to 4:
            if abs(q) <= 4
               and abs(r) <= 4
               and abs(q + r) <= 4:
                H.add((q, r))

    return H

# |H| = 61
```

---

## 3. The six triangular camps

Each camp is an equilateral triangular set of ten holes, seated **flush against one
edge of the central hexagon** so that the union of hexagon and camps forms a
six-pointed star.

The hexagon's edge in the $+q$ direction is the column $q=4$, which contains the
five holes $r\in\{-4,\ldots,0\}$. The camp attached to that edge occupies the four
columns $q=5,\ldots,8$ immediately beyond it, with row counts $4,3,2,1$ decreasing
outward to a single apex at $(8,-4)$:

$$
C_0=
\left\{
(q,r)\in\mathbb Z^2:
5\le q\le 8,
\quad
-4\le r\le -(q-4)
\right\}.
$$

Explicitly,

$$
C_0=
\begin{aligned}
&\{(5,-4),(5,-3),(5,-2),(5,-1),\\
&\ \ (6,-4),(6,-3),(6,-2),\\
&\ \ (7,-4),(7,-3),\\
&\ \ (8,-4)\},
\end{aligned}
$$

whose columns contain

$$
4,3,2,1
$$

holes, for a total of

$$
4+3+2+1=10.
$$

The other five camps are obtained by rotation through multiples of $60^\circ$
(see §4).

> **Note on orientation.** The triangle must point *outward*, i.e. its four-hole
> base lies against the hexagon and its apex points away from the centre. A
> definition whose apex points inward — for example
> $\{(q,r):5\le q\le 8,\ -q+5\le r\le 0\}$ — yields ten holes per camp and a total
> of 121, yet is **not** a Chinese Checkers board: each such triangle meets the
> hexagon in a single hole instead of four, so the six camps hang off the corners
> rather than forming the points of a star. Cardinality checks alone do not detect
> this; see §6 for the contact-count invariant that does.

### Pseudocode

```text
function BaseCamp():
    # Camp C_0: attached to the hexagon edge q = 4,
    # extending outward with column sizes 4, 3, 2, 1.
    base = empty set

    for q from 5 to 8:
        for r from -4 to -(q - 4):
            base.add((q, r))

    assert size(base) == 10
    return base
```

---

## 4. Axial rotation

A $60^\circ$ rotation about the origin in axial coordinates is

$$
R(q,r)=(-r,\;q+r).
$$

It has order six,

$$
R^6(q,r)=(q,r),
$$

so the six camps are the orbit of $C_0$ under $R$:

$$
C_i=R^i(C_0),
\qquad i=0,\ldots,5.
$$

The six camps satisfy $|C_i|=10$ and are pairwise disjoint.

> **Orientation.** $R$ sends $d_0=(1,0)\mapsto(0,1)=d_5$, so it steps *backwards*
> through the direction indexing of §1. Whether this reads as clockwise or
> counter-clockwise on screen depends on the rendering convention: under
> mathematical axes ($y$ upward) it is counter-clockwise, under typical screen
> axes ($y$ downward) it is clockwise. Nothing in the rules depends on the choice
> — only that $R$ has order six and that camps are indexed consistently, since
> $C_{i+3}$ must be the camp diametrically opposite $C_i$.

Because $R^3=-\mathrm{id}$, the opposite-camp relation used throughout is exactly
point reflection through the centre:

$$
C_{(i+3)\bmod 6}=-C_i=\{(-q,-r):(q,r)\in C_i\}.
$$

### Pseudocode

```text
function Rotate60(x):
    (q, r) = x
    return (-r, q + r)

function Rotate(x, n):
    repeat n times:
        x = Rotate60(x)
    return x

function BuildCamps():
    base = BaseCamp()          # see §3: columns q=5..8, sizes 4,3,2,1

    camps = array[6]

    for i from 0 to 5:
        camps[i] = empty set

        for x in base:
            camps[i].add(Rotate(x, i))

    return camps

# For every i:
#   |C_i| = 10
#   C_i = R^i(C_0)
#   C_[(i+3) mod 6] = { (-q,-r) : (q,r) ∈ C_i }
```

---

## 5. Complete board

The complete board is

$$
V=
H_4
\cup
C_0
\cup
C_1
\cup
C_2
\cup
C_3
\cup
C_4
\cup
C_5.
$$

Since the central hexagon and six camps are disjoint,

$$
|V|
=
61+6(10)
=
121.
$$

### Pseudocode

```text
function BuildBoard():
    H = CentralHex()
    camps = BuildCamps()

    V = H.copy()

    for i from 0 to 5:
        V = V ∪ camps[i]

    assert size(V) == 121

    return V, camps
```

---

## 5.1 The board, drawn

Rendering the construction of §§2–5 (digits are camp indices, dots the central
hexagon) must produce a six-pointed star:

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

Each row $r$ is drawn at horizontal offset $2q+r$, which renders axial coordinates as
a hex grid. Note that opposite camps ($0$ and $3$, $1$ and $4$, $2$ and $5$) sit
diametrically across the centre, as §4 requires. Printing this diagram is the
quickest way to catch a mis-specified camp.

---

## 6. Board construction invariants

Every playable coordinate belongs to exactly one of:

- the central hexagon $H_4$, or
- one of the six camps $C_i$.

Thus

$$
V=H_4\ \dot\cup\ C_0\ \dot\cup\ \cdots\ \dot\cup\ C_5,
$$

where $\dot\cup$ denotes disjoint union.

Cardinality alone is a **weak** check: as noted in §3, an inward-pointing camp
definition also gives $|C_i|=10$ and $|V|=121$ while producing a board that is not
a six-pointed star. A correct construction must additionally satisfy the following.

**Camp/hexagon contact.** Each camp's base lies flush against one hexagon edge, so
each camp has exactly four holes adjacent to $H_4$, and the number of camp–hexagon
adjacent pairs is

$$
\left|\{(x,y)\in C_i\times H_4:\ y-x\in D\}\right|=8
$$

for every $i$ (four base holes, each with two hexagon neighbours).

**Connectivity.** The graph $G=(V,E)$ is connected.

**Symmetry.** $V=-V$, and $C_{(i+3)\bmod 6}=-C_i$.

**Uniform initial mobility.** In the initial position every player has the same
number of legal moves (14 on the standard board), which follows from the six-fold
symmetry and is a useful end-to-end check.

### Pseudocode

```text
function ValidateBoard(V, camps, H):
    # Strong invariants — cardinality alone is insufficient.

    assert size(V) == 121
    assert size(H) == 61

    # Disjointness and exact cover
    seen = empty set
    for region in [H] + camps:
        for x in region:
            assert x not in seen
            seen.add(x)
    assert seen == V

    for i from 0 to 5:
        assert size(camps[i]) == 10

        # Each camp meets the hexagon in exactly 8 adjacent pairs
        contacts = 0
        for x in camps[i]:
            for d in D:
                if Add(x, d) in H:
                    contacts += 1
        assert contacts == 8

        # Opposite camp is the point reflection through the centre
        opposite = { (-q, -r) for (q, r) in camps[i] }
        assert camps[(i + 3) mod 6] == opposite

    # Whole board is connected and centrally symmetric
    assert IsConnected(V)
    assert V == { (-q, -r) for (q, r) in V }
```

---

## 7. Players

Define the player set

$$
P=\{0,1,2,3,4,5\}.
$$

Player $i$ starts in camp

$$
C_i
$$

and has target camp

$$
O_i=C_{(i+3)\bmod6}.
$$

### Pseudocode

```text
PLAYERS = {0, 1, 2, 3, 4, 5}

function TargetCamp(player):
    return camps[(player + 3) mod 6]
```

---

## 8. Game state

A game state consists of:

```text
GameState:
    board       : mapping Coordinate → Player | EMPTY
    turn        : Player
    winner      : Player | NONE
    game_over   : Boolean
```

The board satisfies

$$
\forall i\in P:
\quad
|\{x\in V:s(x)=i\}|=10.
$$

### Pseudocode

```text
class GameState:
    board
    turn
    winner
    game_over
```

---

## 9. Initial state

For every $i\in P$,

$$
s_0(x)=i
\qquad\text{for }x\in C_i.
$$

For every remaining board hole,

$$
s_0(x)=\varnothing.
$$

The initial player is player 0:

$$
\operatorname{turn}(s_0)=0.
$$

### Pseudocode

```text
function InitialState(V, camps):
    state = GameState()

    for x in V:
        state.board[x] = EMPTY

    for player from 0 to 5:
        for x in camps[player]:
            state.board[x] = player

    state.turn = 0
    state.winner = NONE
    state.game_over = false

    return state
```

---

## 10. Basic coordinate operations

Define vector addition by

$$
(q,r)+(a,b)=(q+a,r+b).
$$

For $x\in V$ and $d\in D$, define

$$
\operatorname{neighbor}(x,d)=x+d.
$$

A jump has destination

$$
\operatorname{jumpDest}(x,d)=x+2d.
$$

### Pseudocode

```text
function Add(x, y):
    return (x.q + y.q, x.r + y.r)

function Neighbor(x, d):
    return Add(x, d)

function JumpDestination(x, d):
    return Add(x, Scale(d, 2))
```

---

## 11. Adjacent move

Let the active player's piece occupy $x$.

An adjacent move

$$
x\rightarrow y
$$

is legal iff

$$
y=x+d
$$

for some

$$
d\in D,
$$

and

$$
y\in V,
\qquad
s(x)=i,
\qquad
s(y)=\varnothing.
$$

Equivalently,

$$
\boxed{
\operatorname{LegalStep}(s,i,x,y)
\iff
s(x)=i
\land
y-x\in D
\land
s(y)=\varnothing.
}
$$

### Pseudocode

```text
function IsLegalStep(state, player, from, to):
    # LegalStep(s,i,x,y) ⇔
    # s(x) = i ∧ y-x ∈ D ∧ s(y) = EMPTY

    if state.board[from] != player:
        return false

    if to not in V:
        return false

    if to - from not in D:
        return false

    if state.board[to] != EMPTY:
        return false

    return true
```

---

## 12. Single jump

For

$$
d\in D,
$$

a jump

$$
x\rightarrow x+2d
$$

is legal iff

$$
x+d\in V,
$$

$$
s(x+d)\neq\varnothing,
$$

$$
x+2d\in V,
$$

and

$$
s(x+2d)=\varnothing.
$$

For the active player $i$,

$$
\boxed{
\operatorname{LegalJump}(s,i,x,d)
\iff
s(x)=i
\land
x+d\in V
\land
s(x+d)\neq\varnothing
\land
x+2d\in V
\land
s(x+2d)=\varnothing.
}
$$

### Pseudocode

```text
function IsLegalJump(state, player, from, direction):
    # LegalJump(s,i,x,d) ⇔
    # s(x)=i
    # ∧ x+d ∈ V
    # ∧ s(x+d) ≠ EMPTY
    # ∧ x+2d ∈ V
    # ∧ s(x+2d) = EMPTY

    if state.board[from] != player:
        return false

    middle = from + direction
    to     = from + 2 * direction

    if middle not in V:
        return false

    if to not in V:
        return false

    if state.board[middle] == EMPTY:
        return false

    if state.board[to] != EMPTY:
        return false

    return true
```

---

## 13. Applying a jump

If

$$
x\rightarrow y
$$

is a legal jump for player $i$, then

$$
s'(x)=\varnothing,
$$

$$
s'(y)=i,
$$

and

$$
s'(z)=s(z)
\qquad
\forall z\notin\{x,y\}.
$$

The intermediate hole remains unchanged.

### Pseudocode

```text
function ApplyJump(state, player, from, direction):
    # s'(x) = EMPTY
    # s'(x+2d) = i
    # s'(z) = s(z), z ∉ {x, x+2d}
    #
    # The jumped piece at x+d is unchanged.

    to = from + 2 * direction

    new_state = Copy(state)

    new_state.board[from] = EMPTY
    new_state.board[to] = player

    return new_state
```

---

## 14. Jump sequences

A jump move is a finite sequence

$$
x_0,x_1,\ldots,x_k,
\qquad k\ge1,
$$

where

$$
x_{j+1}=x_j+2d_j,
\qquad
d_j\in D.
$$

For every $j$, the jump must be legal in the state resulting from the preceding jumps.

Therefore, if

$$
s_0=s,
$$

and

$$
s_{j+1}
=
\operatorname{ApplyJump}(s_j,i,x_j,d_j),
$$

then

$$
\operatorname{LegalJump}(s_j,i,x_j,d_j)
$$

must hold for every $j$.

The same piece is used throughout the sequence.

### Pseudocode

```text
# A jump sequence:
#
# x₀, x₁, ..., xₖ, k ≥ 1
#
# x[j+1] = x[j] + 2*d[j]
#
# d[j] ∈ D
#
# and:
#
# LegalJump(s[j], player, x[j], d[j])
#
# where s[j+1] is obtained by applying jump j.
#
# The board state is updated after EVERY jump.
```

---

## 15. Generating jumps from a position

Given a piece at $x$, enumerate all six directions.

$$
J(s,i,x)=
\left\{
x+2d:
d\in D,\;
\operatorname{LegalJump}(s,i,x,d)
\right\}.
$$

### Pseudocode

```text
function LegalJumpDestinations(state, player, from):
    destinations = []

    for d in D:
        if IsLegalJump(state, player, from, d):
            destinations.append(from + 2*d)

    return destinations

# J(s,i,x) =
# { x+2d :
#   d ∈ D ∧ LegalJump(s,i,x,d) }
```

---

## 16. Generating all destinations reachable in a jump turn

A subtle but important distinction exists between:

1. **one jump**, and
2. **the set of destinations available to a piece in a whole jump turn**.

After a jump, the piece may jump again, so a turn's reachable set is a transitive
closure, not a single step. Define the jump-reachability relation

$$
x\leadsto_s y
$$

to mean that $y$ is reachable from $x$ by a nonempty legal sequence of jumps. The
player may terminate at any reachable position, so the legal jump destinations are

$$
J^{*}(s,i,x)=\{y: x\leadsto_s y\}.
$$

### 16.1 Key lemma: within one turn, jump legality is position-determined

Let the piece begin the turn at $x_0$ and write

$$
\Omega=\{z\in V: s(z)\neq\varnothing\}\setminus\{x_0\}
$$

for the occupied holes **other than the moving piece**. Because a turn moves only
one piece and jumps never capture, $\Omega$ is *invariant for the entire turn*. When
the piece stands at $x$, the occupancy is exactly $\Omega\cup\{x\}$.

Therefore the jump $x\to x+2d$ is legal at that moment iff

$$
x+d\in V,\quad
x+d\in\Omega,\quad
x+2d\in V,\quad
x+2d\notin\Omega,
$$

which mentions only $x$, $d$ and the fixed set $\Omega$. Note the moving piece can
never be its own blocker or its own landing hole: $x\notin\Omega$, and $x+2d\ne x$
for $d\in D$.

**Consequence.** The jump-successor relation

$$
x\to_{\Omega} x+2d
$$

is a *fixed* directed graph on $V$, determined once per turn. Hence:

$$
J^{*}(s,i,x_0)=\{\text{vertices reachable from }x_0\text{ in }(\;V,\to_\Omega\;)\}\setminus\{x_0\},
$$

and a plain breadth-first or depth-first search over **positions**, using a single
visited set, computes it exactly and always terminates. There is no need to key the
search on board states, and no need for per-path bookkeeping.

Two practical caveats:

- $x_0$ itself may be re-entered (see §18); it is a legal *waypoint* but is only a
  legal *destination* if reached by a nonempty sequence. Conventionally a turn
  ending where it began is excluded, since it is indistinguishable from not moving.
- If the implementation must enumerate jump **paths** (for animation or notation)
  rather than destinations, that is a different, exponential problem; see §18.

### Pseudocode

```text
function JumpDestinations(state, player, start):
    # Exact and terminating: BFS over positions.
    # Omega = occupied holes EXCLUDING the moving piece.

    occupied_others = { z in V : state.board[z] != EMPTY } - { start }

    visited  = { start }
    frontier = [ start ]
    results  = empty set

    while frontier is not empty:
        next_frontier = []

        for cur in frontier:
            for d in D:
                middle = Add(cur, d)
                dest   = Add(cur, Scale(d, 2))

                if middle in V
                   and dest in V
                   and middle in occupied_others
                   and dest not in occupied_others
                   and dest not in visited:

                    visited.add(dest)
                    results.add(dest)          # stopping here is legal
                    next_frontier.append(dest)

        frontier = next_frontier

    return results        # excludes 'start'
```

---

## 17. What is and is not path-dependent

It is tempting to state that jump legality must be recomputed against an evolving
board, and in one narrow sense that is true: the source hole empties and the
destination fills, so

$$
s_{j+1}\neq s_j .
$$

But by the lemma of §16.1 those are the *only* changes, and both concern the moving
piece itself — which is excluded from $\Omega$. So the evolving state carries no
information that affects legality within the turn.

Two claims must therefore be kept apart:

- **Correct.** A jump edge depends on the occupancy of the *other* pieces, so the
  jump graph must be rebuilt at the start of every turn (it changes as the game
  progresses).
- **Incorrect.** "Because occupancy changes after each jump, the search must be
  keyed on $(s,x)$ rather than $x$." Within a single turn $s$ is a function of $x$,
  so $(s,x)$ and $x$ induce the same visited-set semantics — while making the search
  look as though it needs unbounded state.

The practical rule:

```text
# Per TURN: rebuild the jump graph from current occupancy.
#     Omega  <- occupied holes minus the moving piece
#
# Within the turn: search positions with ONE visited set.
#     Do NOT key the search on board states.
```

---

## 18. Termination, revisits, and path enumeration

Jump sequences may change direction and **may genuinely revisit coordinates**. For
example, with a blocker adjacent to the piece, the piece can jump out and
immediately jump back, returning to its starting hole:

$$
(-4,3)\;\longrightarrow\;(-6,3)\;\longrightarrow\;(-4,3).
$$

So the space of legal jump *paths* is infinite, and any procedure that enumerates
paths without a guard will not terminate.

The resolution follows from §16.1:

- To compute the set of **destinations** — which is all the rules require in order
  to generate legal moves — use the position BFS of §16. Revisits are absorbed by
  the visited set; termination is guaranteed after at most $|V|$ expansions.
- To enumerate **paths**, impose an explicit guard, since the problem is otherwise
  unbounded. The natural choice is to forbid repeating a position *within the
  current path*, which keeps every path simple and bounds its length by $|V|$;
  alternatively cap the length. Either way this is a presentational choice, and it
  does **not** change the set of reachable destinations.

> Keying the search on $(s,x)$ — sometimes proposed as the "exact" formulation — is
> not a termination guard at all. Within a turn $s$ is determined by $x$, so
> $(s,x)$ revisits occur exactly when $x$ revisits occur; the pair merely disguises
> the position-visited set while suggesting the state space is larger than it is.

### Pseudocode

```text
# Destinations (what move generation needs): see §16. Terminates.

# Optional: path enumeration, e.g. for move notation or animation.
function JumpPaths(state, player, start, max_len = INFINITY):
    occupied_others = { z in V : state.board[z] != EMPTY } - { start }
    results = []

    function walk(cur, path):
        if length(path) - 1 >= max_len:
            return

        for d in D:
            middle = Add(cur, d)
            dest   = Add(cur, Scale(d, 2))

            if middle in V
               and dest in V
               and middle in occupied_others
               and dest not in occupied_others
               and dest not in path:        # simple-path guard: required

                results.append(path + [dest])
                walk(dest, path + [dest])

    walk(start, [start])
    return results

# NOTE: the simple-path guard makes enumeration finite but is a
# presentational restriction. The reachable DESTINATION set is
# unaffected -- compute it with the BFS of §16.
```

---

## 19. Move representation

Inferring the move type from geometry is fragile, so record it explicitly:

```text
enum MoveType:
    STEP
    JUMP

Move:
    type        : STEP | JUMP
    origin      : Coordinate
    destination : Coordinate
    route       : list[Coordinate] | NONE   # optional, presentational
```

**Identity.** A move is identified by `(type, origin, destination)`. By §21 the
resulting position depends only on those three fields, so `route` must be excluded
from equality and hashing — otherwise the same move reached by two routes compares
unequal and move lists acquire duplicates.

For a `STEP`, `destination` is adjacent to `origin` and `route` is unused. For a
`JUMP`, `destination` lies in $J^{*}$ (§16); `route`, when present, is one legal
sequence of jump holes from `origin` to `destination`, useful for animation or
notation but never for legality or equality.

Note that `route` may revisit holes (§18), and that its length is not determined by
the move: several routes of differing lengths may share a destination.

### Pseudocode

```text
class Move:
    type
    origin
    destination
    route = NONE

    function key():
        # route deliberately excluded
        return (type, origin, destination)

    function equals(other):
        return key() == other.key()
```

---

## 20. Legal move generation

For active player $i$, the complete legal move set is

$$
M(s,i)
=
M_{\text{step}}(s,i)
\cup
M_{\text{jump}}(s,i).
$$

The step moves are

$$
M_{\text{step}}(s,i)
=
\left\{
(x,y):
s(x)=i,\;
y-x\in D,\;
s(y)=\varnothing
\right\}.
$$

The jump moves are, for each $i$-piece at $x$, the reachable destinations of §16:

$$
M_{\text{jump}}(s,i)
=
\left\{
(x,y):
s(x)=i,\;
y\in J^{*}(s,i,x)
\right\}.
$$

A move is identified by its **(origin, destination)** pair, not by the route taken.
Distinct jump routes to the same destination produce the same resulting position and
must therefore count as one move — otherwise move counts and any search built on
them are inflated by duplicates.

### Pseudocode

```text
function LegalMoves(state, player):
    moves = []

    for x in V:

        if state.board[x] != player:
            continue

        # ---- Single adjacent steps ----

        for d in D:
            to = Add(x, d)

            if IsLegalStep(state, player, x, to):
                moves.append(
                    Move(type = STEP, origin = x, destination = to)
                )

        # ---- Jump turns: one move per reachable destination ----

        for dest in JumpDestinations(state, player, x):
            moves.append(
                Move(type = JUMP, origin = x, destination = dest)
            )

    return moves

# JumpDestinations returns a SET, so each destination yields exactly
# one move regardless of how many routes reach it.
#
# If a concrete route is needed (animation, notation), recover one
# by recording BFS predecessors, or call JumpPaths (§18) separately.
```

> **Note.** If `Move.path` is required to record the full route rather than just the
> destination, store a single representative route per destination (e.g. the BFS
> shortest one) and keep deduplication by destination. See §19.

---

## 21. Applying a complete move

For an adjacent move

$$
x\rightarrow y,
$$

apply one state transition.

For a jump turn ending at $y$, the net effect on the board is the same regardless of
the route taken:

$$
s'(x)=\varnothing,
\qquad
s'(y)=i,
\qquad
s'(z)=s(z)\ \ \forall z\notin\{x,y\},
$$

because no intermediate hole is ever modified and no piece is captured. Applying a
route hole-by-hole and applying the net displacement therefore agree — which is why
deduplicating jump moves by destination (§20) is sound.

### Pseudocode

```text
function ApplyMove(state, move):
    player = state.turn
    result = Copy(state)

    origin      = move.origin
    destination = move.destination

    if move.type == STEP:
        assert IsLegalStep(result, player, origin, destination)

    else if move.type == JUMP:
        # Validate against the turn's jump graph (§16).
        assert destination in JumpDestinations(result, player, origin)

        # If move.route is recorded, optionally verify each hop:
        if move.route != NONE:
            current = origin
            for hop in move.route:
                direction = Divide(Subtract(hop, current), 2)
                assert direction in D
                assert IsLegalJump(result, player, current, direction)
                # Intermediate holes are NEVER modified; only the
                # moving piece relocates.
                result.board[current] = EMPTY
                result.board[hop] = player
                current = hop
            return result

    # Net effect: vacate origin, occupy destination.
    result.board[origin] = EMPTY
    result.board[destination] = player

    return result
```

---

## 22. Turn advancement

After a legal move by player $i$, if nobody has won, the next player is

$$
i'=(i+1)\bmod6.
$$

### Pseudocode

```text
function AdvanceTurn(state):
    state.turn = (state.turn + 1) mod 6
```

---

## 23. Winning condition

Player $i$'s target camp is

$$
O_i=C_{(i+3)\bmod6}.
$$

Player $i$ wins iff

$$
\forall x\in O_i,\qquad s(x)=i.
$$

Define

$$
\operatorname{Won}(s,i)
\iff
\forall x\in C_{(i+3)\bmod6},
\ s(x)=i.
$$

### Pseudocode

```text
function HasWon(state, player):
    # Won(s,i) ⇔
    # ∀x ∈ C[(i+3) mod 6], s(x) = i

    target = camps[(player + 3) mod 6]

    for x in target:
        if state.board[x] != player:
            return false

    return true
```

---

## 24. Completing a turn

After applying a move:

1. test whether the active player has won;
2. if so, terminate;
3. otherwise advance the turn.

### Pseudocode

```text
function ExecuteMove(state, move):

    player = state.turn

    assert not state.game_over

    legal_moves = LegalMoves(state, player)

    assert move ∈ legal_moves

    state = ApplyMove(state, move)

    if HasWon(state, player):
        state.winner = player
        state.game_over = true
        return state

    state.turn = (player + 1) mod 6

    return state
```

---

## 25. Core game loop

A player with no legal move must be handled explicitly: it is **reachable**, not
impossible. A player whose ten pieces fill a camp can be sealed in by opponents
occupying the five frontier holes of that camp together with the corresponding jump
landing holes, leaving zero legal moves. Such a position is legal under the rules of
this specification, so an unconditional `assert size(moves) > 0` is unsound.

This specification resolves it by **passing**: a player with no legal move forfeits
the turn and play continues. If every player in a full cycle passes, the position is
frozen and the game is a draw.

> Rule sets differ here. Passing is the least intrusive choice, but some rule sets
> instead forbid the blocking configuration, or oblige the blocking player to move
> aside. Whatever the choice, it must be explicit; see §30 for parameterisation.

### Pseudocode

```text
function PlayGame():
    V, camps = BuildBoard()
    ValidateBoard(V, camps, CentralHex())      # §6

    state = InitialState(V, camps)
    consecutive_passes = 0

    while not state.game_over:

        player = state.turn
        moves  = LegalMoves(state, player)

        if size(moves) == 0:
            # Reachable: a fully blocked player passes.
            consecutive_passes += 1

            if consecutive_passes == 6:
                # Nobody can move: frozen position.
                state.game_over = true
                state.winner = NONE
                return NONE

            state.turn = (player + 1) mod 6
            continue

        consecutive_passes = 0

        move  = ChooseMove(state, moves, player)
        state = ExecuteMove(state, move)

    return state.winner
```

---

## 26. Formal game tuple

The complete game can be represented as

$$
\mathcal G=
(V,D,C,s_0,T,W),
$$

where:

$$
V\subset\mathbb Z^2,
\qquad |V|=121,
$$

$$
D=
\{
(1,0),(1,-1),(0,-1),
(-1,0),(-1,1),(0,1)
\},
$$

$$
C=(C_0,C_1,C_2,C_3,C_4,C_5),
$$

$$
|C_i|=10,
$$

$$
s_0(x)=i
\quad\text{for }x\in C_i,
$$

and

$$
T_i
$$

is the transition relation consisting of exactly one legal adjacent move or one nonempty legal jump sequence by player $i$.

The winning set for player $i$ is

$$
W_i=
\left\{
s:
\forall x\in C_{(i+3)\bmod6},
\ s(x)=i
\right\}.
$$

---

## 27. Move legality — complete mathematical definition

For player $i$, a **step move**

$$
(x,y)
$$

is legal iff

$$
\boxed{
s(x)=i
\land
y-x\in D
\land
s(y)=\varnothing.
}
$$

A **jump**

$$
(x,x+2d)
$$

is legal in state $s$ iff

$$
\boxed{
s(x)=i
\land
d\in D
\land
x+d\in V
\land
s(x+d)\neq\varnothing
\land
x+2d\in V
\land
s(x+2d)=\varnothing.
}
$$

A **jump sequence**

$$
x_0,x_1,\ldots,x_k
$$

is legal iff

$$
\boxed{
k\ge1
}
$$

and, for every

$$
j=0,\ldots,k-1,
$$

there exists

$$
d_j\in D
$$

such that

$$
\boxed{
x_{j+1}=x_j+2d_j
}
$$

and

$$
\boxed{
s_j(x_j+d_j)\neq\varnothing,
\qquad
s_j(x_{j+1})=\varnothing,
}
$$

where

$$
s_{j+1}
=
\operatorname{ApplyJump}(s_j,x_j,d_j).
$$

The complete turn is either exactly one step or exactly one such nonempty jump sequence.

---

## 28. Invariants

A correct implementation should preserve the following invariants after every completed move.

### Piece count

For every player $i$,

$$
\boxed{
|\{x\in V:s(x)=i\}|=10.
}
$$

### Board capacity

For every $x\in V$,

$$
\boxed{
s(x)\in\{\varnothing,0,1,2,3,4,5\}.
}
$$

### Total occupancy

$$
\boxed{
|\{x:s(x)\neq\varnothing\}|=60.
}
$$

### Empty holes

$$
\boxed{
|\{x:s(x)=\varnothing\}|=61.
}
$$

### Piece conservation

No move changes the ownership or number of pieces.

### Pseudocode

```text
function ValidateState(state):
    occupied = 0

    for player from 0 to 5:
        count = 0

        for x in V:
            if state.board[x] == player:
                count += 1
                occupied += 1

        assert count == 10

    assert occupied == 60

    empty = 0

    for x in V:
        if state.board[x] == EMPTY:
            empty += 1

    assert empty == 61
```

---

## 29. Implementation notes

The following distinctions are essential.

### A. A jump is not a capture

The intermediate piece remains on the board.

### B. Jumping is not restricted by ownership

The intermediate piece may belong to any player.

### C. Multi-jumps use the evolving state

Every subsequent jump is evaluated after the preceding jump has modified occupancy.

### D. The same piece performs the entire jump chain

A turn cannot switch pieces.

### E. Direction may change

If the first jump uses $d_0$, the next may use any

$$
d_1\in D.
$$

There is no straight-line requirement.

### F. The player may stop after any jump

A jump sequence of length $k$ is a complete legal move for every

$$
k\ge1
$$

for which the sequence is legal. The player is not required to continue jumping.

### G. No capture occurs

The number of pieces is invariant.

---

## 30. Camp-rule parameterization

The specification above uses the **unrestricted camp convention**: camps impose no special movement restrictions.

If a particular ruleset requires additional restrictions, make them an explicit legality predicate rather than embedding them into the geometric movement rules.

Define

$$
\operatorname{CampLegal}(s,i,\text{move})
$$

and require

$$
\operatorname{LegalMove}(s,i,\text{move})
\iff
\operatorname{GeometricallyLegal}(s,i,\text{move})
\land
\operatorname{CampLegal}(s,i,\text{move}).
$$

Under the unrestricted ruleset,

$$
\boxed{
\operatorname{CampLegal}\equiv\text{true}.
}
$$

### Pseudocode

```text
function CampLegal(state, player, move):
    # Unrestricted ruleset:
    #
    # CampLegal(s, i, move) = TRUE

    return true
```

---

## 31. Minimal reference API

An implementation can expose the game through the following interface:

```text
Board:
    coordinates()
    camps()
    neighbors(position)
    is_valid(position)

GameState:
    board
    turn
    winner
    game_over

Move:
    type
    origin
    destination
    route

Game:
    initial_state()
    legal_moves(state)
    apply_move(state, move)
    has_won(state, player)
    is_terminal(state)
```

The fundamental operations are:

```text
V, camps = BuildBoard()

state = InitialState(V, camps)

moves = LegalMoves(state, state.turn)

state = ExecuteMove(state, move)

terminal = state.game_over
winner   = state.winner
```

---

## 32. One-line normative rule

The entire movement system reduces to the following.

> Move one of your own pieces either **one adjacent step** into an empty hole, or
> through a **nonempty sequence of jumps**, each hopping over an occupied hole into
> the empty hole directly beyond it. The same piece performs every jump, no piece is
> ever captured, and the first player to occupy all ten holes of the opposite camp
> wins.

The board is exactly

$$
V=H_4\ \dot\cup\ C_0\ \dot\cup\ C_1\ \dot\cup\ C_2\ \dot\cup\ C_3\ \dot\cup\ C_4\ \dot\cup\ C_5,
\qquad |V|=121,
$$

with central hexagon

$$
H_4=\{(q,r)\in\mathbb Z^2:\ |q|\le4,\ |r|\le4,\ |q+r|\le4\},
\qquad |H_4|=61,
$$

base camp (apex pointing **outward**, base flush to the hexagon edge $q=4$)

$$
C_0=\{(q,r)\in\mathbb Z^2:\ 5\le q\le 8,\ -4\le r\le-(q-4)\},
\qquad |C_0|=10,
$$

the remaining camps given by $C_i=R^i(C_0)$ for $R(q,r)=(-r,q+r)$, and directions

$$
D=\{(1,0),(1,-1),(0,-1),(-1,0),(-1,1),(0,1)\}.
$$

The target of player $i$ is

$$
O_i=C_{(i+3)\bmod 6}=-C_i.
$$

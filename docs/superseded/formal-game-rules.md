# Chinese Checkers — Formal Game Rules

This specification defines the standard six-player game of **Chinese Checkers** as a mathematical state-transition system. It is intended to be sufficiently precise for implementation.

## 1. Board

Chinese Checkers is played on a six-pointed star containing exactly **121 playable holes**.

Represent the board as a finite set

$$
V \subset \mathbb{Z}^2,
\qquad |V|=121.
$$

The six allowed adjacency directions are

$$
D =
\{
(1,0),(1,-1),(0,-1),
(-1,0),(-1,1),(0,1)
\}.
$$

The listing is in rotational order, so consecutive elements are $60^\circ$ apart and
$-d\in D$ for every $d\in D$. Only the set matters to the rules; the ordering is
fixed here so that direction indices agree with the companion *Implementation
Specification*.

Two holes $u,v\in V$ are adjacent iff

$$
v-u\in D.
$$

Equivalently, the board graph is

$$
G=(V,E),
$$

where

$$
\{u,v\}\in E
\iff
v-u\in D.
$$

The board consists of:

- a central hexagonal region containing 61 holes;
- six triangular camps, each containing 10 holes.

Let the six camps be

$$
C_0,C_1,\ldots,C_5,
$$

with

$$
C_i\subset V,
\qquad
|C_i|=10,
$$

and the indices taken cyclically modulo 6.

The camp opposite $C_i$ is

$$
C_{(i+3)\bmod 6}.
$$

Geometrically, each camp is a triangle of ten holes whose four-hole base lies flush
against one edge of the central hexagon and whose apex points outward, so that
$V$ forms a six-pointed star. Equivalently, indexing the camps in rotational order,
the opposite camp is the point reflection of $C_i$ through the centre:

$$
C_{(i+3)\bmod 6}=-C_i=\{-v: v\in C_i\}.
$$

A concrete realisation in axial coordinates — including the exact camp definition and
the invariants that distinguish a genuine star from a merely 121-hole
look-alike — is given in the companion *Implementation Specification*, §§2–6.

### Pseudocode

```text
# Board:
# V ⊆ Z², |V| = 121
#
# D = {(1,0), (1,-1), (0,-1), (-1,0), (-1,1), (0,1)}
#
# Adjacent(u, v) ⇔ v - u ∈ D
#
# G = (V, E), where {u,v} ∈ E ⇔ v-u ∈ D
#
# |C_i| = 10 for each i ∈ {0,...,5}
# OppositeCamp(i) = C[(i + 3) mod 6]
```

---

## 2. Players and pieces

There are six players,

$$
P=\{0,1,2,3,4,5\}.
$$

Each player owns exactly ten indistinguishable pieces.

A game position is an occupancy function

$$
s:V\rightarrow P\cup\{\varnothing\}.
$$

For every player $i$,

$$
\left|\{v\in V:s(v)=i\}\right|=10.
$$

Every hole contains at most one piece.

Consequently, every valid position contains

$$
6\cdot10=60
$$

pieces and

$$
121-60=61
$$

empty holes.

### Pseudocode

```text
# State:
# s : V → {0,1,2,3,4,5, EMPTY}
#
# For every player i:
# |{v ∈ V : s[v] = i}| = 10
#
# Exactly 60 holes are occupied and 61 are empty.
```

---

## 3. Initial position

Player $i$'s ten pieces initially occupy camp $C_i$:

$$
s_0(v)=i
\qquad
\text{for every }v\in C_i.
$$

Every other hole is empty:

$$
s_0(v)=\varnothing
\qquad
\text{for }v\notin\bigcup_{i=0}^{5}C_i.
$$

### Pseudocode

```text
# Initial state:
#
# s₀(v) = i  for every v ∈ C_i
#
# s₀(v) = EMPTY
# for every v not belonging to any camp.
```

---

## 4. Legal movement

A turn moves exactly **one** piece belonging to the active player.

There are two mutually exclusive types of movement:

1. an adjacent move;
2. a sequence of one or more jumps.

An adjacent move cannot be followed by a jump during the same turn.

---

## 4.1 Adjacent move

A piece at $u$ may move directly to $v$ iff

$$
v-u\in D
$$

and

$$
s(v)=\varnothing.
$$

Thus the destination must be an adjacent board hole and must be empty.

The resulting state $s'$ satisfies

$$
s'(u)=\varnothing,
\qquad
s'(v)=i,
$$

with all other occupancies unchanged.

### Pseudocode

```text
# Adjacent move:
#
# LegalAdjacent(u, v) ⇔
#     v ∈ V
#     ∧ v - u ∈ D
#     ∧ s[u] = i
#     ∧ s[v] = EMPTY
#
# Equivalently:
# LegalAdjacent(u, v) ⇔
#     v-u ∈ D ∧ s(v)=EMPTY
# for a piece owned by player i at u.
#
# Result:
#     s'(u) = EMPTY
#     s'(v) = i
#     s'(x) = s(x) for all x ≠ u,v
```

---

## 5. Jump movement

Let a piece currently occupy $u$.

For any direction $d\in D$, it may jump from $u$ to

$$
u+2d
$$

iff:

1. $u+d$ is a board hole;
2. $u+d$ is occupied;
3. $u+2d$ is a board hole;
4. $u+2d$ is empty.

Formally,

$$
u+d\in V,
$$

$$
s(u+d)\neq\varnothing,
$$

and

$$
u+2d\in V,
\qquad
s(u+2d)=\varnothing.
$$

The jumped piece is **not captured or removed**.

Thus a jump is

$$
u\longrightarrow u+2d.
$$

### Pseudocode

```text
# Jump:
#
# For d ∈ D:
#
# LegalJump(u, d) ⇔
#     u + d ∈ V
#     ∧ s[u + d] ≠ EMPTY
#     ∧ u + 2d ∈ V
#     ∧ s[u + 2d] = EMPTY
#
# Equivalently:
#
# LegalJump(u, d) ⇔
#     s(u+d) ≠ ∅
#     ∧ s(u+2d) = ∅
#
# with u+d, u+2d ∈ V.
#
# Result of the jump:
#     s'(u)     = EMPTY
#     s'(u+2d)  = s(u)
#
# The piece at u+d is unchanged.
# No capture occurs.
```

---

## 6. Multiple jumps

A turn may consist of an arbitrary finite sequence of jumps performed by the **same piece**.

Suppose the piece follows

$$
u_0,u_1,\ldots,u_k,
\qquad k\ge1.
$$

The sequence is legal iff for every $j\in\{0,\ldots,k-1\}$, there exists

$$
d_j\in D
$$

such that

$$
u_{j+1}=u_j+2d_j,
$$

and, immediately before that jump,

$$
u_j+d_j
$$

is occupied while

$$
u_j+2d_j
$$

is empty.

Thus

$$
u_{j+1}-u_j=2d_j,
\qquad d_j\in D.
$$

Importantly, legality is evaluated against the **current state after all preceding jumps**, not against the state at the beginning of the turn.

That said, the dependence is weaker than it appears. Because a turn moves only one
piece and jumps never capture, the occupancy of every *other* hole is constant
throughout the turn. Writing

$$
\Omega=\{z\in V:s(z)\neq\varnothing\}\setminus\{u_0\}
$$

for the occupied holes excluding the moving piece, $\Omega$ is invariant for the
whole turn, and the $j$-th jump is legal iff

$$
u_j+d_j\in\Omega,
\qquad
u_j+2d_j\in V\setminus\Omega .
$$

Only $u_j$, $d_j$ and the fixed set $\Omega$ appear. So within a single turn the set
of available jumps depends solely on the piece's **current position**, and the
reachable destinations form the forward closure of a directed graph that is fixed
once per turn. The moving piece can never block itself, since $u_0\notin\Omega$.

The player may stop the jump sequence after any legal jump. Note that a jump
sequence may revisit holes, including $u_0$; a turn is conventionally required to
end somewhere other than where it began, since ending at $u_0$ is indistinguishable
from not moving.

### Pseudocode

```text
# Multi-jump:
#
# A jump sequence is
#
#     u₀, u₁, ..., uₖ,  k ≥ 1
#
# with
#
#     u[j+1] = u[j] + 2*d[j]
#     d[j] ∈ D
#
# and, immediately before every jump j:
#
#     s(u[j] + d[j]) ≠ EMPTY
#     s(u[j] + 2*d[j]) = EMPTY
#
# IMPORTANT:
#     The state is updated after every jump.
#
# Therefore the legality of jump j is evaluated
# against the state produced by jumps 0,...,j-1.
#
# The player may terminate the sequence after any
# legal jump; continuation is optional.
```

---

## 7. Same-piece restriction

A multi-jump turn cannot switch pieces.

If the first jump moves a piece from $u_0$ to $u_1$, every subsequent jump in that turn must originate at the piece's new location.

Thus

$$
u_0\rightarrow u_1\rightarrow\cdots\rightarrow u_k
$$

describes the trajectory of one physical piece.

It is not legal to jump one piece and then jump another piece during the same turn.

### Pseudocode

```text
# Once a jump turn starts:
#
# moving_piece = the piece selected for the first jump
#
# Every subsequent jump must originate from
# the current position of moving_piece.
#
# A turn may NOT:
#
#     jump(piece A)
#     jump(piece B)
#
# The same piece must perform the entire jump chain.
```

---

## 8. Jump direction

The direction may change between successive jumps.

For example,

$$
d_0\neq d_1
$$

is completely legal.

There is no requirement that a jump sequence continue in a straight line.

### Pseudocode

```text
# Successive jumps may use arbitrary directions:
#
# d[j] ∈ D independently for every j.
#
# There is no requirement that
#
#     d[j+1] = d[j].
```

---

## 9. Jumping over pieces

The jumped piece may belong to either the moving player or an opponent.

If the local configuration is

$$
A\;B\;\square
$$

along one of the six board directions, then $A$ may jump over $B$, producing

$$
\square\;B\;A.
$$

No ownership condition is imposed on $B$.

### Pseudocode

```text
# The intermediate piece may belong to ANY player.
#
# LegalJump depends only on:
#
#     s[u+d] ≠ EMPTY
#
# not on the identity of s[u+d].
#
# Therefore both own and opponent pieces may be jumped over.
```

---

## 10. No captures

Jumping never removes the jumped piece.

If a piece at $u$ jumps over a piece at $u+d$, then

$$
s'(u+d)=s(u+d).
$$

The only occupancy changes are:

$$
s'(u)=\varnothing,
$$

and

$$
s'(u+2d)=s(u).
$$

### Pseudocode

```text
# Jumping does NOT capture.
#
# If piece A jumps over piece B:
#
#     source becomes EMPTY
#     destination becomes A
#     B remains unchanged
#
# s'(u)    = EMPTY
# s'(u+d)  = s(u+d)
# s'(u+2d) = s(u)
```

---

## 11. Complete turn

For active player $i$, a legal turn is exactly one of:

### Type A — adjacent move

$$
u\rightarrow v
$$

where $v-u\in D$ and $v$ is empty.

### Type B — jump sequence

$$
u_0\rightarrow u_1\rightarrow\cdots\rightarrow u_k,
\qquad k\ge1,
$$

where every transition is a legal jump by the same piece.

No other form of turn is legal.

In particular:

- zero movement is not a turn;
- two different pieces cannot move in one turn;
- an adjacent move followed by a jump is illegal;
- a jump followed by an adjacent move is illegal;
- after a jump, continuing to jump is optional.

### Pseudocode

```text
# LegalTurn(i) is either:
#
#   1. one legal adjacent move by an i-piece
#
# OR
#
#   2. one or more legal jumps by one i-piece.
#
# No mixed movement is allowed.
#
# Formally:
#
# Turn ∈ {
#     one adjacent move,
#     jump₁, jump₂, ..., jumpₖ where k ≥ 1
# }
#
# and all jumps belong to the same piece.
```

---

## 12. Turn order

For the six-player game, turns proceed cyclically:

$$
0,1,2,3,4,5,0,1,\ldots
$$

Equivalently, if $t$ is the zero-based turn number, the active player is

$$
i_t=t\bmod 6,
$$

**provided every player always has a legal move.** That proviso does not hold in
general: a fully blocked player passes (§17), which advances the player index without
consuming a turn in the above sense. Implementations should therefore track the
active player as explicit state rather than deriving it from a turn counter.

The game terminates when a player satisfies the winning condition, or when all six
players pass in succession (§17).

### Pseudocode

```text
# Active player on turn t:
#
#     i = t mod 6
#
# Turn order:
#
#     0 → 1 → 2 → 3 → 4 → 5 → 0 → ...
```

---

## 13. Objective

Player $i$ must move all ten of their pieces into the camp opposite their starting camp.

Define

$$
O_i=C_{(i+3)\bmod 6}.
$$

Player $i$ has won iff

$$
\forall v\in O_i,\quad s(v)=i.
$$

Because

$$
|O_i|=10
$$

and player $i$ has exactly ten pieces, this is equivalent to saying that all ten of player $i$'s pieces occupy the opposite camp.

### Pseudocode

```text
# Target camp:
#
#     O_i = C[(i + 3) mod 6]
#
# Player i wins iff
#
#     ∀v ∈ O_i : s[v] = i
#
# Since |O_i| = 10 and player i has exactly
# ten pieces, this means all ten pieces occupy O_i.
```

---

## 14. Winning state

Define the winning-state set for player $i$ as

$$
W_i=
\left\{
s:
\forall v\in C_{(i+3)\bmod 6},\ s(v)=i
\right\}.
$$

The game ends at the first state belonging to some $W_i$.

Formally, if

$$
s_0,s_1,s_2,\ldots
$$

is the sequence of states produced by play, the winner is the first player $i$ such that

$$
s_t\in W_i
$$

for some $t$.

### Pseudocode

```text
# Winning-state set:
#
# W_i = { s :
#         ∀v ∈ C[(i+3) mod 6],
#         s(v) = i
#       }
#
# After every completed turn:
#
#     if IsWinningState(i, state):
#         winner = i
#         game_over = true
```

---

## 15. State-transition formulation

Let $S$ be the set of all valid positions.

For each player $i$, define the legal-turn relation

$$
T_i\subseteq S\times S.
$$

A pair

$$
(s,s')\in T_i
$$

iff $s'$ can be obtained from $s$ by one legal turn of player $i$.

The game is therefore a finite alternating state-transition system

$$
s_0
\overset{0}{\longrightarrow}
s_1
\overset{1}{\longrightarrow}
s_2
\overset{2}{\longrightarrow}
\cdots
$$

where

$$
(s_t,s_{t+1})\in T_{t\bmod6}.
$$

### Pseudocode

```text
# State-transition system:
#
# T_i ⊆ S × S
#
# (s, s') ∈ T_i
#     ⇔ s' is reachable from s by one legal turn
#        of player i.
#
# Game trajectory:
#
#     s₀ --0--> s₁ --1--> s₂ --2--> ...
#
# with:
#
#     (s_t, s[t+1]) ∈ T[t mod 6]
```

---

## 16. Camp movement convention

The core movement rules above permit pieces to enter, leave, and move within any camp, subject only to the ordinary movement rules.

Some physical sets, online implementations, and tournament/house rules impose additional **camp restrictions**, particularly concerning occupation of an opponent's camp.

Such restrictions are not universal to all rule sets. Therefore an implementation must explicitly choose its camp convention.

Under the unrestricted convention used by this specification:

$$
\text{camp membership imposes no additional movement constraint.}
$$

### Pseudocode

```text
# Camp convention used here:
#
# There are NO special movement restrictions
# associated with camps.
#
# A piece may enter, leave, or move within any camp
# provided the ordinary movement rules are satisfied.
#
# If a specific ruleset imposes camp restrictions,
# those restrictions must be implemented separately.
```

---

## 17. Players with no legal move

The rules as stated do **not** guarantee that the active player has a legal move.
The condition is reachable: if player $i$'s ten pieces fill a camp, opponents may
occupy that camp's five frontier holes together with the holes beyond them, so that
no step and no jump is available. Player $i$ then has

$$
T_i(s)=\varnothing
$$

while still holding all ten pieces, so this is neither a win nor a loss under §13.

This specification resolves the case by **passing**: a player with no legal move
forfeits the turn, and play passes to the next player. Consequently the active
player on turn $t$ is no longer simply $t\bmod 6$ — §12's formula applies only when
every player always has a move.

If all six players pass in succession the position cannot change, and the game is
declared a **draw**.

$$
T_i(s)=\varnothing\ \ \forall i
\quad\Longrightarrow\quad
\text{draw}.
$$

> Rule sets differ here. Passing is the least intrusive convention; others forbid
> the blocking configuration outright, or require the blocking player to move aside.
> Like the camp convention of §16, this choice must be made explicitly.

### Pseudocode

```text
# A player with no legal move PASSES.
#
#   if LegalTurns(s, i) is empty:
#       skip player i
#
# If six consecutive players pass, the position is
# frozen and the game is a draw.
#
# NOTE: therefore active_player != t mod 6 in general.
```

---

## 18. Compact normative definition

A **Chinese Checkers turn** is the movement of exactly one own piece, either

- **one adjacent step** into an empty hole, or
- **one or more consecutive jumps**, each over an occupied hole into the empty hole
  directly beyond it.

For a jump in direction $d\in D$,

$$
\boxed{
u\rightarrow u+2d
}
$$

is legal exactly when

$$
\boxed{
u+d\in V,\quad
s(u+d)\neq\varnothing,\quad
u+2d\in V,\quad
s(u+2d)=\varnothing.
}
$$

The jumped piece is never removed.

The same piece performs every jump in a multi-jump sequence, directions may change between jumps, and the player may stop after any jump.

For six players, turns occur cyclically according to

$$
\boxed{
i_t=t\bmod6.
}
$$

Player $i$ wins when

$$
\boxed{
\forall v\in C_{(i+3)\bmod 6},\quad s(v)=i.
}
$$

Thus the game can be represented as

$$
\boxed{
\mathcal G=
(V,D,\{C_i\}_{i=0}^{5},s_0,\{T_i\}_{i=0}^{5},\{W_i\}_{i=0}^{5})
}
$$

with the definitions above.

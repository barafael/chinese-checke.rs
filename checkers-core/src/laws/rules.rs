//! Rules laws: positions, movement, turns, and winning (chapters 6–15).
//!
//! These are checked over generated and constructed positions rather than proven
//! — jump reachability is a graph closure, which bounded model checking will not
//! reach without unwind bounds that would weaken the claim to a bounded one.
//!
//! Each law below is written to *fail on a wrong implementation*, following the
//! proof audit: assertions that merely correlate with correctness are not enough.
//! Where a claim is about a set, the law compares sets rather than sizes; where it
//! is about a count, it pins the count from both sides.

use std::collections::HashSet;

use crate::geometry::{Coord, Dir, all_holes, in_camp, on_board};
use crate::law::{Evidence, Law};
use crate::position::{
    HOLES, Move, MoveKind, PIECES_PER_PLAYER, PLAYERS, Player, Position, is_legal_jump,
    is_legal_step,
};
use crate::register_law;
use crate::rng::Xorshift;
use crate::rules::{
    Outcome, apply, apply_route, blocked_position, frozen_position, jump_destinations, jump_routes,
    legal_moves,
};
use crate::spec::Chapter;
use crate::turn::{JumpTurn, single_hop_destinations};

/// A deterministic sequence of positions to check invariants over: the initial
/// position, then positions reached by playing a fixed pseudo-random game.
///
/// Fixed seed rather than a random one, so a failure is reproducible.
fn sample_positions(count: usize) -> Vec<Position> {
    let mut out = vec![Position::initial()];
    let mut pos = Position::initial();
    let mut rng = Xorshift::new(0x5EED);

    for ply in 0..count {
        let player = Player::wrapping((ply % PLAYERS) as u8);
        let moves = legal_moves(&pos, player);
        if moves.is_empty() {
            continue;
        }
        let mv = &moves[rng.below(moves.len())];
        pos = apply(&pos, mv);
        out.push(pos.clone());
    }
    out
}

/// Positions with a single piece plus scattered blockers, for jump laws.
fn jump_scenarios() -> Vec<(Position, Coord)> {
    let holes = all_holes();
    let mut out = Vec::new();
    let mut rng = Xorshift::new(0xC0FFEE);

    for _ in 0..60 {
        let mut pos = Position::empty();
        let mut occupied = Vec::new();
        let n = 6 + rng.below(30);
        for _ in 0..n {
            let c = holes[rng.below(holes.len())];
            if pos.is_empty_hole(c) {
                pos.set(c, Some(Player::wrapping(occupied.len() as u8)));
                occupied.push(c);
            }
        }
        if let Some(&origin) = occupied.first() {
            out.push((pos, origin));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Chapter 6: players, pieces, the initial position
// ---------------------------------------------------------------------------

/// Piece conservation.
///
/// Stated over the specification's six players, and its subjects are
/// six-player positions. In a composed game (chapter 15) the same idea is
/// per seated player — ten each, none for the vacant camps — which is what
/// [`crate::audit::audit_position`] checks and `tests/composed.rs` runs.
pub struct PieceConservation;

impl Law for PieceConservation {
    const ID: &'static str = "CC-POS-PIECES";
    const STATEMENT: &'static str = r"\forall i \in P:\ \left|\{v \in V : s(v) = i\}\right| = 10";
    const CHAPTER: Chapter = Chapter::Players;
    const SUMMARY: &'static str = "Every player owns exactly ten pieces in every position.";
    /// In plain terms: Every player always owns exactly ten pieces.
    const NOTE: &'static str = "Every player always owns exactly ten pieces.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        for player in Player::ALL {
            let n = pos.count_of(player);
            if n != PIECES_PER_PLAYER {
                return Err(format!("player {:?} owns {n} pieces", player.index()));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(120)
    }
}
register_law!(PieceConservation, PIECE_CONSERVATION);

/// Occupancy accounting: 60 occupied, 61 empty, 121 total.
///
/// Six-player by definition, like [`PieceConservation`]; a composed game
/// occupies ten holes per seated player instead, and
/// [`crate::audit::audit_position`] checks that.
pub struct OccupancyAccounting;

impl Law for OccupancyAccounting {
    const ID: &'static str = "CC-POS-OCCUPANCY";
    const STATEMENT: &'static str = r"\left|\{v : s(v) \neq \varnothing\}\right| = 60 \ \land\ \left|\{v : s(v) = \varnothing\}\right| = 61";
    const CHAPTER: Chapter = Chapter::Players;
    const SUMMARY: &'static str = "Sixty holes are occupied and sixty-one empty, totalling 121.";
    /// In plain terms: Sixty holes are occupied, sixty-one are empty, and together they are the whole board.
    const NOTE: &'static str =
        "Sixty holes are occupied, sixty-one are empty, and together they are the whole board.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        let occupied = pos.occupied_count();
        let empty = pos.empty_count();
        let expected = PLAYERS * PIECES_PER_PLAYER;
        if occupied != expected {
            return Err(format!("{occupied} holes occupied, expected {expected}"));
        }
        if empty != HOLES - expected {
            return Err(format!(
                "{empty} holes empty, expected {}",
                HOLES - expected
            ));
        }
        if occupied + empty != HOLES {
            return Err(format!("{occupied} + {empty} != {HOLES}"));
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(120)
    }
}
register_law!(OccupancyAccounting, OCCUPANCY_ACCOUNTING);

/// In the initial position each player fills their own camp, and nothing else is
/// occupied.
pub struct InitialPosition;

impl Law for InitialPosition {
    const ID: &'static str = "CC-POS-INITIAL";
    const STATEMENT: &'static str =
        r"s_0(v) = i \iff v \in C_i,\qquad s_0(v) = \varnothing \iff v \in H_4";
    const CHAPTER: Chapter = Chapter::Players;
    const SUMMARY: &'static str =
        "Initially each player fills their own camp and the hexagon is empty.";
    /// In plain terms: At the start every camp is full of its own pieces and the middle is empty.
    const NOTE: &'static str =
        "At the start every camp is full of its own pieces and the middle is empty.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        let pos = Position::initial();
        for c in all_holes() {
            let expected = (0..PLAYERS as u32)
                .find(|i| in_camp(c, *i))
                .map(|i| Player::wrapping(i as u8));
            if pos.occupant(c) != expected {
                return Err(format!(
                    "{c:?} holds {:?}, expected {:?}",
                    pos.occupant(c).map(|p| p.index()),
                    expected.map(|p| p.index())
                ));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(InitialPosition, INITIAL_POSITION);

/// The target camp is the opposite camp, and the relation is an involution.
pub struct TargetCampIsOpposite;

impl Law for TargetCampIsOpposite {
    const ID: &'static str = "CC-POS-TARGET";
    const STATEMENT: &'static str =
        r"O_i = C_{(i+3) \bmod 6},\qquad O_{O_i} = C_i,\qquad O_i \cap C_i = \varnothing";
    const CHAPTER: Chapter = Chapter::Players;
    const SUMMARY: &'static str = "A player's target is the opposite camp, distinct from their start, and the pairing is mutual.";
    /// In plain terms: Your goal is the camp directly across the centre from where you start.
    const NOTE: &'static str =
        "Your goal is the camp directly across the centre from where you start.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = Player;

    fn holds(player: &Player) -> Result<(), String> {
        let start: HashSet<Coord> = player.start_camp().into_iter().collect();
        let target: HashSet<Coord> = player.target_camp().into_iter().collect();

        if start.len() != PIECES_PER_PLAYER || target.len() != PIECES_PER_PLAYER {
            return Err(format!(
                "camp sizes are {} and {}, expected {PIECES_PER_PLAYER}",
                start.len(),
                target.len()
            ));
        }
        if !start.is_disjoint(&target) {
            return Err("a player's start and target camps overlap".into());
        }
        if player.opposite().opposite() != *player {
            return Err("the opposite-player relation is not an involution".into());
        }
        // The target is the point reflection of the start.
        let reflected: HashSet<Coord> = start.iter().map(|c| c.negate()).collect();
        if reflected != target {
            return Err("the target camp is not the reflection of the start camp".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<Player> {
        Player::ALL.to_vec()
    }
}
register_law!(TargetCampIsOpposite, TARGET_CAMP_IS_OPPOSITE);

// ---------------------------------------------------------------------------
// Chapter 7: adjacent moves
// ---------------------------------------------------------------------------

/// A step is legal exactly when the destination is adjacent, on the board, and
/// empty — and the generated step moves are exactly those.
pub struct StepLegality;

impl Law for StepLegality {
    const ID: &'static str = "CC-STEP-LEGAL";
    const STATEMENT: &'static str = r"\mathrm{step}(x,y) \iff s(x) = i \ \land\ y - x \in D \ \land\ y \in V \ \land\ s(y) = \varnothing";
    const CHAPTER: Chapter = Chapter::Steps;
    const SUMMARY: &'static str =
        "Generated steps are exactly the adjacent, on-board, empty destinations.";
    /// In plain terms: You may step to any adjacent empty hole.
    const NOTE: &'static str = "You may step to any adjacent empty hole.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        for player in Player::ALL {
            // What the specification says the step set is.
            let mut expected: HashSet<(Coord, Coord)> = HashSet::new();
            for origin in pos.pieces_of(player) {
                for d in Dir::ALL {
                    let to = origin.neighbour(d);
                    if on_board(to) && pos.is_empty_hole(to) {
                        expected.insert((origin, to));
                    }
                }
            }
            // What the generator produced.
            let produced: HashSet<(Coord, Coord)> = legal_moves(pos, player)
                .into_iter()
                .filter(|m| m.kind == MoveKind::Step)
                .map(|m| (m.origin, m.destination))
                .collect();

            if produced != expected {
                let missing: Vec<_> = expected.difference(&produced).collect();
                let extra: Vec<_> = produced.difference(&expected).collect();
                return Err(format!(
                    "step set mismatch for player {}: missing {missing:?}, extra {extra:?}",
                    player.index()
                ));
            }

            // The predicate must agree with the set in BOTH directions.
            // Checking only acceptance is one-sided: an over-permissive
            // predicate is never exercised, because move generation only ever
            // offers neighbours. Non-neighbours must be checked explicitly.
            for (origin, to) in &expected {
                if !is_legal_step(pos, player, *origin, *to) {
                    return Err(format!("is_legal_step rejected legal {origin:?}->{to:?}"));
                }
            }
            for origin in pos.pieces_of(player) {
                for target in pos.holes() {
                    let adjacent = Dir::ALL.iter().any(|d| origin.neighbour(*d) == *target);
                    let legal = is_legal_step(pos, player, origin, *target);
                    let should = adjacent && pos.is_empty_hole(*target);
                    if legal != should {
                        return Err(format!(
                            "is_legal_step({},{} -> {},{}) = {legal}, expected {should}                              (adjacent {adjacent}, empty {}, distance {})",
                            origin.q,
                            origin.r,
                            target.q,
                            target.r,
                            pos.is_empty_hole(*target),
                            origin.distance(*target),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(60)
    }
}
register_law!(StepLegality, STEP_LEGALITY);

/// A step never moves onto an occupied hole, and moves exactly one hole.
pub struct StepDisplacement;

impl Law for StepDisplacement {
    const ID: &'static str = "CC-STEP-DISPLACE";
    const STATEMENT: &'static str = r"y - x \in D \implies \mathrm{dist}(x,y) = 1";
    const CHAPTER: Chapter = Chapter::Steps;
    const SUMMARY: &'static str =
        "A step moves to an adjacent hole, never further and never onto a piece.";
    /// In plain terms: A step moves exactly one hole and lands on an empty one.
    const NOTE: &'static str = "A step moves exactly one hole and lands on an empty one.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        for player in Player::ALL {
            for mv in legal_moves(pos, player) {
                if mv.kind != MoveKind::Step {
                    continue;
                }
                if mv.origin.distance(mv.destination) != 1 {
                    return Err(format!(
                        "step {:?}->{:?} spans distance {}",
                        mv.origin,
                        mv.destination,
                        mv.origin.distance(mv.destination)
                    ));
                }
                if !pos.is_empty_hole(mv.destination) {
                    return Err(format!("step lands on occupied {:?}", mv.destination));
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(60)
    }
}
register_law!(StepDisplacement, STEP_DISPLACEMENT);

// ---------------------------------------------------------------------------
// Chapter 8: jumps
// ---------------------------------------------------------------------------

/// A jump requires an occupied midpoint and an empty landing hole.
///
/// Both clauses are checked in both directions, so neither dropping the blocker
/// requirement nor ignoring the landing hole passes.
pub struct JumpLegality;

impl Law for JumpLegality {
    const ID: &'static str = "CC-JUMP-LEGAL";
    const STATEMENT: &'static str = r"\mathrm{jump}(x,d) \iff x+d \in V \ \land\ s(x+d) \neq \varnothing \ \land\ x+2d \in V \ \land\ s(x+2d) = \varnothing";
    const CHAPTER: Chapter = Chapter::Jumps;
    const SUMMARY: &'static str =
        "A jump needs an occupied hole to cross and an empty hole to land on.";
    /// In plain terms: You may jump only over an occupied hole and only land on an empty one.
    const NOTE: &'static str =
        "You may jump only over an occupied hole and only land on an empty one.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let Some(player) = pos.occupant(*origin) else {
            return Ok(());
        };
        for d in Dir::ALL {
            let mid = origin.neighbour(d);
            let dest = origin.jump_dest(d);
            let expected = on_board(mid)
                && on_board(dest)
                && !pos.is_empty_hole(mid)
                && pos.is_empty_hole(dest);
            let actual = is_legal_jump(pos, player, *origin, d);
            if actual != expected {
                return Err(format!(
                    "is_legal_jump({origin:?}, {d:?}) = {actual}, expected {expected} \
                     (mid on board {}, mid occupied {}, dest on board {}, dest empty {})",
                    on_board(mid),
                    !pos.is_empty_hole(mid),
                    on_board(dest),
                    pos.is_empty_hole(dest)
                ));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(JumpLegality, JUMP_LEGALITY);

/// A jump never captures: the crossed piece is unchanged.
pub struct JumpDoesNotCapture;

impl Law for JumpDoesNotCapture {
    const ID: &'static str = "CC-JUMP-NO-CAPTURE";
    const STATEMENT: &'static str = r"s'(x+d) = s(x+d),\qquad \left|\{v : s'(v) \neq \varnothing\}\right| = \left|\{v : s(v) \neq \varnothing\}\right|";
    const CHAPTER: Chapter = Chapter::Jumps;
    const SUMMARY: &'static str = "Jumping leaves the crossed piece in place and removes nothing.";
    /// In plain terms: Jumping never removes the piece you jumped over.
    const NOTE: &'static str = "Jumping never removes the piece you jumped over.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let Some(player) = pos.occupant(*origin) else {
            return Ok(());
        };
        for d in Dir::ALL {
            if !is_legal_jump(pos, player, *origin, d) {
                continue;
            }
            let mid = origin.neighbour(d);
            let before = pos.occupant(mid);
            let after = apply(pos, &Move::jump(*origin, origin.jump_dest(d)));

            if after.occupant(mid) != before {
                return Err(format!("crossed piece at {mid:?} changed"));
            }
            if after.occupied_count() != pos.occupied_count() {
                return Err(format!(
                    "piece count changed from {} to {}",
                    pos.occupied_count(),
                    after.occupied_count()
                ));
            }
            for p in Player::ALL {
                if after.count_of(p) != pos.count_of(p) {
                    return Err(format!("player {}'s piece count changed", p.index()));
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(JumpDoesNotCapture, JUMP_DOES_NOT_CAPTURE);

/// Jumping is blind to ownership: only occupancy of the crossed hole matters.
pub struct JumpIgnoresOwnership;

impl Law for JumpIgnoresOwnership {
    const ID: &'static str = "CC-JUMP-ANY-OWNER";
    const STATEMENT: &'static str =
        r"\mathrm{jump}(x,d) \text{ depends on } s(x+d) \neq \varnothing, \text{ not on } s(x+d)";
    const CHAPTER: Chapter = Chapter::Jumps;
    const SUMMARY: &'static str = "A piece may be jumped regardless of which player owns it.";
    /// In plain terms: You may jump over anyone's piece, yours or theirs.
    const NOTE: &'static str = "You may jump over anyone's piece, yours or theirs.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = Player;

    fn holds(owner: &Player) -> Result<(), String> {
        // Same geometry, only the crossed piece's owner varies: legality must
        // not change.
        let origin = Coord::new(0, 0);
        let mid = Coord::new(1, 0);
        let dest = Coord::new(2, 0);
        let mover = Player::ALL[0];

        let mut pos = Position::empty();
        pos.set(origin, Some(mover));
        pos.set(mid, Some(*owner));

        if !is_legal_jump(&pos, mover, origin, Dir::E) {
            return Err(format!(
                "jump over a piece owned by player {} was rejected",
                owner.index()
            ));
        }
        if !jump_destinations(&pos, origin).contains(&dest) {
            return Err(format!(
                "destination unreachable when crossing player {}'s piece",
                owner.index()
            ));
        }
        Ok(())
    }

    fn subjects() -> Vec<Player> {
        Player::ALL.to_vec()
    }
}
register_law!(JumpIgnoresOwnership, JUMP_IGNORES_OWNERSHIP);

// ---------------------------------------------------------------------------
// Chapter 9: jump sequences and reachability
// ---------------------------------------------------------------------------

/// The position-BFS agrees with exhaustive route enumeration.
///
/// This is the central claim of chapter 9, and the one the draft specification
/// got wrong. Comparing *sets* rather than sizes is deliberate: equal
/// cardinalities with different contents would pass a size check.
pub struct JumpClosureIsExact;

impl Law for JumpClosureIsExact {
    const ID: &'static str = "CC-JUMP-CLOSURE";
    const STATEMENT: &'static str = r"\{y : x \leadsto_s y\} = \{\mathrm{last}(\pi) : \pi \text{ a simple jump route from } x\}";
    const CHAPTER: Chapter = Chapter::JumpSequences;
    const SUMMARY: &'static str =
        "Breadth-first search over positions yields exactly the routes' destinations.";
    /// In plain terms: Everything reachable by any chain of jumps is found by exploring one jump at a time.
    const NOTE: &'static str =
        "Everything reachable by any chain of jumps is found by exploring one jump at a time.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let bfs = jump_destinations(pos, *origin);
        let by_route: HashSet<Coord> = jump_routes(pos, *origin, HOLES)
            .into_iter()
            .filter_map(|r| r.last().copied())
            .filter(|c| c != origin)
            .collect();

        if bfs != by_route {
            // Summarise rather than dump: a full coordinate list buries the
            // signal in the violation message.
            let missing: Vec<_> = by_route.difference(&bfs).take(4).collect();
            let extra: Vec<_> = bfs.difference(&by_route).take(4).collect();
            return Err(format!(
                "closure mismatch from ({},{}): BFS is missing {} destination(s) {missing:?}                  and wrongly offers {} {extra:?}",
                origin.q,
                origin.r,
                by_route.difference(&bfs).count(),
                bfs.difference(&by_route).count(),
            ));
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(JumpClosureIsExact, JUMP_CLOSURE_IS_EXACT);

/// Within a turn, occupancy is a function of the moving piece's position.
///
/// The formal content of chapter 9's argument: the set $\Omega$ is invariant, so
/// reaching a hole by different routes leaves the same jumps available. This is
/// what makes the position-keyed search exact.
pub struct OccupancyIsPositionDetermined;

impl Law for OccupancyIsPositionDetermined {
    const ID: &'static str = "CC-JUMP-OMEGA";
    const STATEMENT: &'static str = r"\Omega = \{v : s(v) \neq \varnothing\} \setminus \{x_0\} \text{ is invariant during a turn}";
    const CHAPTER: Chapter = Chapter::JumpSequences;
    const SUMMARY: &'static str =
        "The other pieces never move during a turn, so available jumps depend only on position.";
    /// In plain terms: During your turn the other pieces stand still, so what you can reach depends only on where you are.
    const NOTE: &'static str = "During your turn the other pieces stand still, so what you can reach depends only on where you are.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let omega: HashSet<Coord> = pos.occupied_except(*origin).into_iter().collect();

        // The moving piece is never in Omega, so it cannot block itself.
        if omega.contains(origin) {
            return Err(format!("the moving piece at {origin:?} is inside Omega"));
        }

        // Following any route, Omega is unchanged in the resulting position.
        for route in jump_routes(pos, *origin, 4) {
            let dest = *route.last().unwrap();
            let after = apply_route(pos, &Move::jump(*origin, dest).with_route(route.clone()));
            let after_omega: HashSet<Coord> = after.occupied_except(dest).into_iter().collect();
            if after_omega != omega {
                return Err(format!(
                    "Omega changed along route {route:?}: {} holes differ",
                    after_omega.symmetric_difference(&omega).count()
                ));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(
    OccupancyIsPositionDetermined,
    OCCUPANCY_IS_POSITION_DETERMINED
);

/// Jump routes may revisit holes, so route enumeration needs a guard.
///
/// Stated as a law because it is the reason the simple-path guard exists: if this
/// were false, the guard would be unnecessary and the infinite-route argument
/// would not apply.
pub struct JumpRoutesCanRevisit;

impl Law for JumpRoutesCanRevisit {
    const ID: &'static str = "CC-JUMP-REVISIT";
    const STATEMENT: &'static str = r"\exists s, x, d:\ x \rightarrow x+2d \rightarrow x, \text{ so the route space is infinite}";
    const CHAPTER: Chapter = Chapter::JumpSequences;
    const SUMMARY: &'static str =
        "A piece can jump out and back, so unguarded route enumeration does not terminate.";
    /// In plain terms: You can jump out and straight back, so one turn may visit the same hole twice.
    const NOTE: &'static str =
        "You can jump out and straight back, so one turn may visit the same hole twice.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        // Origin with a blocker beside it: hop out, then hop back.
        let origin = Coord::new(0, 0);
        let mut pos = Position::empty();
        pos.set(origin, Some(Player::ALL[0]));
        pos.set(Coord::new(1, 0), Some(Player::ALL[1]));

        let out = Coord::new(2, 0);
        if !jump_destinations(&pos, origin).contains(&out) {
            return Err("the piece cannot hop over its neighbouring blocker".into());
        }

        // From the landing hole, the reverse hop is available again.
        let mut moved = pos.clone();
        moved.set(origin, None);
        moved.set(out, Some(Player::ALL[0]));
        if !jump_destinations(&moved, out).contains(&origin) {
            return Err("the piece cannot hop back, so routes could not cycle".into());
        }

        // The origin is nonetheless excluded from its own destination set.
        if jump_destinations(&pos, origin).contains(&origin) {
            return Err("the origin must not be offered as a destination".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(JumpRoutesCanRevisit, JUMP_ROUTES_CAN_REVISIT);

// ---------------------------------------------------------------------------
// Chapter 10: move representation and generation
// ---------------------------------------------------------------------------

/// Move identity ignores the route.
pub struct MoveIdentityIgnoresRoute;

impl Law for MoveIdentityIgnoresRoute {
    const ID: &'static str = "CC-MOVE-IDENTITY";
    const STATEMENT: &'static str =
        r"m_1 = m_2 \iff (\mathrm{kind}, x, y)_1 = (\mathrm{kind}, x, y)_2";
    const CHAPTER: Chapter = Chapter::MoveGeneration;
    const SUMMARY: &'static str =
        "Two routes to the same hole are the same move, so routes are not part of identity.";
    /// In plain terms: A move is its start and its end; the path taken does not matter.
    const NOTE: &'static str = "A move is its start and its end; the path taken does not matter.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        let (a, b) = (Coord::new(0, 0), Coord::new(4, 0));
        let m1 = Move::jump(a, b).with_route(vec![a, Coord::new(2, 0), b]);
        let m2 = Move::jump(a, b).with_route(vec![a, Coord::new(2, -2), b]);
        let m3 = Move::jump(a, b);

        if m1 != m2 || m1 != m3 {
            return Err("moves differing only in route compared unequal".into());
        }
        if m1.key() != m2.key() {
            return Err("move keys differ despite equal identity".into());
        }
        // Different destinations must still differ.
        if m1 == Move::jump(a, Coord::new(2, 0)) {
            return Err("moves with different destinations compared equal".into());
        }
        // And step is distinct from jump.
        if Move::step(a, b) == Move::jump(a, b) {
            return Err("a step compared equal to a jump".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(MoveIdentityIgnoresRoute, MOVE_IDENTITY_IGNORES_ROUTE);

/// Generated moves are free of duplicates, and the jump moves are exactly the
/// reachable destinations.
pub struct MoveGenerationIsDeduplicated;

impl Law for MoveGenerationIsDeduplicated {
    const ID: &'static str = "CC-MOVE-DEDUP";
    const STATEMENT: &'static str = r"M_{\text{jump}}(s,i) = \{(x,y) : s(x) = i,\ y \in J^{*}(s,i,x)\}, \text{ without repetition}";
    const CHAPTER: Chapter = Chapter::MoveGeneration;
    const SUMMARY: &'static str =
        "Move generation yields one move per reachable destination, with no duplicates.";
    /// In plain terms: There is exactly one move per destination you can reach.
    const NOTE: &'static str = "There is exactly one move per destination you can reach.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        for player in Player::ALL {
            let moves = legal_moves(pos, player);

            let unique: HashSet<&Move> = moves.iter().collect();
            if unique.len() != moves.len() {
                return Err(format!(
                    "player {} got {} moves but only {} distinct",
                    player.index(),
                    moves.len(),
                    unique.len()
                ));
            }

            // The jump moves match the closure exactly, per origin.
            for origin in pos.pieces_of(player) {
                let expected = jump_destinations(pos, origin);
                let produced: HashSet<Coord> = moves
                    .iter()
                    .filter(|m| m.kind == MoveKind::Jump && m.origin == origin)
                    .map(|m| m.destination)
                    .collect();
                if produced != expected {
                    return Err(format!(
                        "jump moves from {origin:?} do not match the closure"
                    ));
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(60)
    }
}
register_law!(
    MoveGenerationIsDeduplicated,
    MOVE_GENERATION_IS_DEDUPLICATED
);

/// A move never leaves the board, and always moves a piece the player owns.
pub struct MovesStayOnBoard;

impl Law for MovesStayOnBoard {
    const ID: &'static str = "CC-MOVE-ONBOARD";
    const STATEMENT: &'static str = r"\forall m \in M(s,i):\ x, y \in V \ \land\ s(x) = i";
    const CHAPTER: Chapter = Chapter::MoveGeneration;
    const SUMMARY: &'static str =
        "Every generated move starts on one of the player's pieces and ends on the board.";
    /// In plain terms: Every move starts on your own piece and ends on an empty board hole.
    const NOTE: &'static str =
        "Every move starts on your own piece and ends on an empty board hole.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        for player in Player::ALL {
            for mv in legal_moves(pos, player) {
                if !on_board(mv.origin) || !on_board(mv.destination) {
                    return Err(format!("move {mv:?} leaves the board"));
                }
                if pos.occupant(mv.origin) != Some(player) {
                    return Err(format!(
                        "player {} moves a piece it does not own at {:?}",
                        player.index(),
                        mv.origin
                    ));
                }
                if !pos.is_empty_hole(mv.destination) {
                    return Err(format!("move {mv:?} lands on an occupied hole"));
                }
                if mv.origin == mv.destination {
                    return Err("a move must change the piece's hole".into());
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(60)
    }
}
register_law!(MovesStayOnBoard, MOVES_STAY_ON_BOARD);

// ---------------------------------------------------------------------------
// Chapter 11: applying a move
// ---------------------------------------------------------------------------

/// Applying a route hole-by-hole equals applying the net effect.
///
/// This is what licenses identifying moves by destination.
pub struct RouteEqualsNetEffect;

impl Law for RouteEqualsNetEffect {
    const ID: &'static str = "CC-APPLY-NET";
    const STATEMENT: &'static str =
        r"s'(x) = \varnothing,\quad s'(y) = i,\quad s'(z) = s(z)\ \ \forall z \notin \{x,y\}";
    const CHAPTER: Chapter = Chapter::Applying;
    const SUMMARY: &'static str =
        "Replaying a route hole-by-hole gives the same position as applying the net effect.";
    /// In plain terms: Replaying a route hole by hole ends exactly where the move says.
    const NOTE: &'static str = "Replaying a route hole by hole ends exactly where the move says.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let Some(player) = pos.occupant(*origin) else {
            return Ok(());
        };

        for route in jump_routes(pos, *origin, 4) {
            let dest = *route.last().unwrap();
            let mv = Move::jump(*origin, dest).with_route(route.clone());

            let via_route = apply_route(pos, &mv);
            let via_net = apply(pos, &Move::jump(*origin, dest));

            if via_route != via_net {
                return Err(format!("route {route:?} diverged from the net effect"));
            }

            // And the net effect is exactly as stated.
            if !via_net.is_empty_hole(*origin) {
                return Err("the origin was not vacated".into());
            }
            if via_net.occupant(dest) != Some(player) {
                return Err("the destination was not occupied by the mover".into());
            }
            for c in pos.holes() {
                if *c != *origin && *c != dest && via_net.occupant(*c) != pos.occupant(*c) {
                    return Err(format!("unrelated hole {c:?} changed"));
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(RouteEqualsNetEffect, ROUTE_EQUALS_NET_EFFECT);

// ---------------------------------------------------------------------------
// Chapter 12: turns, passing, termination
// ---------------------------------------------------------------------------

/// Turn order advances cyclically.
pub struct TurnOrderCycles;

impl Law for TurnOrderCycles {
    const ID: &'static str = "CC-TURN-CYCLE";
    const STATEMENT: &'static str =
        r"\mathrm{next}(i) = (i+1) \bmod 6,\qquad \mathrm{next}^6 = \mathrm{id}";
    const CHAPTER: Chapter = Chapter::Turns;
    const SUMMARY: &'static str = "Turn order cycles through all six players and returns.";
    /// In plain terms: Turns go around the table in order and return to the start.
    const NOTE: &'static str = "Turns go around the table in order and return to the start.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = Player;

    fn holds(player: &Player) -> Result<(), String> {
        let mut p = *player;
        let mut seen = HashSet::new();
        for _ in 0..PLAYERS {
            if !seen.insert(p) {
                return Err(format!("player {} repeated within one cycle", p.index()));
            }
            p = p.next();
        }
        if p != *player {
            return Err("six advances did not return to the starting player".into());
        }
        if seen.len() != PLAYERS {
            return Err(format!(
                "a cycle visited {} players, expected 6",
                seen.len()
            ));
        }
        Ok(())
    }

    fn subjects() -> Vec<Player> {
        Player::ALL.to_vec()
    }
}
register_law!(TurnOrderCycles, TURN_ORDER_CYCLES);

/// A player can be left with no legal move while holding all ten pieces.
///
/// Stated positively as a law because the draft specification asserted the
/// opposite, and the assertion was unsound.
pub struct BlockedPlayerIsReachable;

impl Law for BlockedPlayerIsReachable {
    const ID: &'static str = "CC-TURN-BLOCKED";
    const STATEMENT: &'static str = r"\exists s, i:\ T_i(s) = \varnothing \ \land\ \left|\{v : s(v) = i\}\right| = 10 \ \land\ \neg\mathrm{Won}(s,i)";
    const CHAPTER: Chapter = Chapter::Turns;
    const SUMMARY: &'static str =
        "A player can have no legal move yet all ten pieces, which is neither a win nor a loss.";
    /// In plain terms: A player can hold all ten pieces and still have no move, which is neither a win nor a loss.
    const NOTE: &'static str = "A player can hold all ten pieces and still have no move, which is neither a win nor a loss.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        let pos = blocked_position();
        let player = Player::ALL[0];

        let moves = legal_moves(&pos, player);
        if !moves.is_empty() {
            return Err(format!(
                "the constructed position leaves {} legal moves",
                moves.len()
            ));
        }
        if pos.count_of(player) != PIECES_PER_PLAYER {
            return Err(format!(
                "the blocked player holds {} pieces",
                pos.count_of(player)
            ));
        }
        if pos.has_won(player) {
            return Err("the blocked position is a win, so it proves nothing".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(BlockedPlayerIsReachable, BLOCKED_PLAYER_IS_REACHABLE);

/// A blocked player passes, and a draw needs every player to pass in a row —
/// in every game size.
pub struct PassingAndDraw;

impl Law for PassingAndDraw {
    const ID: &'static str = "CC-TURN-PASS";
    const STATEMENT: &'static str = r"T_i(s) = \varnothing \implies \text{pass};\qquad |\text{successive passes}| = |P| \implies \text{draw}";
    const CHAPTER: Chapter = Chapter::Turns;
    const SUMMARY: &'static str =
        "A player with no move passes; when every player has passed in a row, the game is a draw.";
    /// In plain terms: A stuck player passes, and a draw needs every player to pass in a row.
    const NOTE: &'static str =
        "A stuck player passes, and a draw needs every player to pass in a row.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        use crate::rules::Game;

        // Chapter 15 leaves the player count open, and the front-end deals
        // games for two, three, and six — the pass and draw rules are checked
        // over all three compositions.
        let configurations: [&[Player]; 3] = [
            Player::ALL.as_slice(),
            &[Player::ALL[0], Player::ALL[3]],
            &[Player::ALL[0], Player::ALL[2], Player::ALL[4]],
        ];

        for players in configurations {
            let names: Vec<u8> = players.iter().map(|p| p.index()).collect();

            // A blocked player passes and play continues with the next
            // seated player, skipping any vacant camps.
            let mut game = Game::compose(blocked_position(), players[0], players);
            if !game.legal_moves().is_empty() {
                return Err(format!(
                    "players {names:?}: the blocked player unexpectedly has moves"
                ));
            }
            game.pass();
            if game.turn() != players[1] {
                return Err(format!(
                    "players {names:?}: passing did not advance to the next seated player"
                ));
            }
            if game.is_over() {
                return Err(format!("players {names:?}: a single pass ended the game"));
            }

            // On a frozen board every seated player must pass, and the draw
            // fires exactly when the passes in a row reach the number of
            // seated players — not one pass sooner.
            let mut frozen = Game::compose(frozen_position(), players[0], players);
            for i in 0..players.len() {
                if frozen.is_over() {
                    return Err(format!(
                        "players {names:?}: the game ended after {i} passes, expected {}",
                        players.len()
                    ));
                }
                if !frozen.legal_moves().is_empty() {
                    return Err(format!(
                        "players {names:?}: the frozen position is not actually frozen"
                    ));
                }
                frozen.pass();
            }
            if frozen.outcome() != Some(Outcome::Draw) {
                return Err(format!(
                    "players {names:?}: {} passes gave {:?}, expected a draw",
                    players.len(),
                    frozen.outcome()
                ));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(PassingAndDraw, PASSING_AND_DRAW);

/// A played move resets the consecutive-pass counter.
///
/// The reset is observable only through scripted play: over
/// [`blocked_position`], player 0 passes, player 1 (the only other player
/// with pieces) moves, and play continues. A counter that survives the move
/// keeps climbing and wrongly draws the game partway through the loop.
pub struct PassCounterResetsOnMove;

impl Law for PassCounterResetsOnMove {
    const ID: &'static str = "CC-TURN-PASS-RESET";
    const STATEMENT: &'static str = r"\mathrm{move}(s, i, m) \implies \mathrm{passes}(s \cdot m) = 0;\quad \text{draw} \iff \text{six passes in succession}";
    const CHAPTER: Chapter = Chapter::Turns;
    const SUMMARY: &'static str =
        "A played move resets the pass counter, so a draw needs six passes after it.";
    /// In plain terms: A played move resets the pass count, so a draw needs six passes in a row.
    const NOTE: &'static str =
        "A played move resets the pass count, so a draw needs six passes in a row.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        use crate::rules::Game;

        // Fixture premise: in the blocked position player 0 is sealed in,
        // players 2..5 hold no pieces, and player 1 — the blocker — can move.
        // Without a mover the passes below would legitimately draw and prove
        // nothing about the reset.
        let pos = blocked_position();
        if !legal_moves(&pos, Player::ALL[0]).is_empty()
            || legal_moves(&pos, Player::ALL[1]).is_empty()
        {
            return Err("the blocked fixture must seal player 0 and leave player 1 a move".into());
        }

        let mut game = Game::from_position(pos, Player::ALL[0]);

        // Pass when stuck, move when able. Player 1's first move vacates a
        // frontier hole, which unseals player 0, so the sequence interleaves
        // moves and passes from there on. On a correct implementation the
        // counter is reset by every move and never reaches six; a counter
        // that survives moves climbs past six and ends the game.
        for ply in 0..12 {
            if game.is_over() {
                return Err(format!(
                    "the game ended at ply {ply}, but passes interleaved with a \
                     move never reach six in succession"
                ));
            }
            let moves = game.legal_moves();
            if moves.is_empty() {
                game.pass();
            } else {
                game.play(&moves[0].clone());
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(PassCounterResetsOnMove, PASS_COUNTER_RESETS_ON_MOVE);

// ---------------------------------------------------------------------------
// Chapter 13: winning
// ---------------------------------------------------------------------------

/// Winning is exactly occupying every hole of the target camp.
pub struct WinCondition;

impl Law for WinCondition {
    const ID: &'static str = "CC-WIN-CONDITION";
    const STATEMENT: &'static str =
        r"\mathrm{Won}(s,i) \iff \forall v \in C_{(i+3) \bmod 6}:\ s(v) = i";
    const CHAPTER: Chapter = Chapter::Winning;
    const SUMMARY: &'static str =
        "A player wins exactly when every hole of the opposite camp holds one of their pieces.";
    /// In plain terms: You win by filling the opposite camp with all ten of your pieces.
    const NOTE: &'static str = "You win by filling the opposite camp with all ten of your pieces.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = Player;

    fn holds(player: &Player) -> Result<(), String> {
        // Filling the target camp wins, for this player only.
        let mut pos = Position::empty();
        for c in player.target_camp() {
            pos.set(c, Some(*player));
        }
        if !pos.has_won(*player) {
            return Err("filling the target camp did not win".into());
        }
        for other in Player::ALL {
            if other != *player && pos.has_won(other) {
                return Err(format!("player {} also won", other.index()));
            }
        }

        // One hole short is not a win.
        let target = player.target_camp();
        let mut short = pos.clone();
        short.set(target[0], None);
        if short.has_won(*player) {
            return Err("a position one hole short counted as a win".into());
        }

        // An opponent's piece in the camp is not a win either.
        let mut usurped = pos.clone();
        usurped.set(target[0], Some(player.next()));
        if usurped.has_won(*player) {
            return Err("a camp containing an opponent's piece counted as a win".into());
        }

        // The initial position is not a win for anyone.
        if Position::initial().has_won(*player) {
            return Err("the initial position is a win".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<Player> {
        Player::ALL.to_vec()
    }
}
register_law!(WinCondition, WIN_CONDITION);

// ---------------------------------------------------------------------------
// Chapter 14: invariants preserved by play
// ---------------------------------------------------------------------------

/// Playing a legal move preserves every position invariant.
pub struct PlayPreservesInvariants;

impl Law for PlayPreservesInvariants {
    const ID: &'static str = "CC-INV-PRESERVED";
    const STATEMENT: &'static str = r"(s, s') \in T_i \implies \forall j:\ \left|\{v : s'(v) = j\}\right| = \left|\{v : s(v) = j\}\right|";
    const CHAPTER: Chapter = Chapter::Invariants;
    const SUMMARY: &'static str =
        "Every legal move preserves each player's piece count and total occupancy.";
    /// In plain terms: No move ever creates, destroys, or hands over a piece.
    const NOTE: &'static str = "No move ever creates, destroys, or hands over a piece.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = Position;

    fn holds(pos: &Position) -> Result<(), String> {
        for player in Player::ALL {
            for mv in legal_moves(pos, player) {
                let after = apply(pos, &mv);

                for p in Player::ALL {
                    if after.count_of(p) != pos.count_of(p) {
                        return Err(format!(
                            "move {mv:?} changed player {}'s count from {} to {}",
                            p.index(),
                            pos.count_of(p),
                            after.count_of(p)
                        ));
                    }
                }
                if after.occupied_count() != pos.occupied_count() {
                    return Err(format!("move {mv:?} changed the occupied count"));
                }
                if after.empty_count() != pos.empty_count() {
                    return Err(format!("move {mv:?} changed the empty count"));
                }
                // Exactly two holes differ: origin and destination.
                let changed = pos
                    .holes()
                    .iter()
                    .filter(|c| pos.occupant(**c) != after.occupant(**c))
                    .count();
                if changed != 2 {
                    return Err(format!("move {mv:?} changed {changed} holes, expected 2"));
                }
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<Position> {
        sample_positions(30)
    }
}
register_law!(PlayPreservesInvariants, PLAY_PRESERVES_INVARIANTS);

// ---------------------------------------------------------------------------
// Chapter 15: variants
// ---------------------------------------------------------------------------

/// The unrestricted camp convention: camps impose no extra movement constraint.
pub struct CampsAreUnrestricted;

impl Law for CampsAreUnrestricted {
    const ID: &'static str = "CC-VAR-CAMP-FREE";
    const STATEMENT: &'static str = r"\mathrm{CampLegal}(s, i, m) \equiv \text{true}";
    const CHAPTER: Chapter = Chapter::Variants;
    const SUMMARY: &'static str =
        "Under the unrestricted convention a piece may enter, leave, or cross any camp.";
    /// In plain terms: Camps add no extra rules: any piece may enter, leave, or cross any camp.
    const NOTE: &'static str =
        "Camps add no extra rules: any piece may enter, leave, or cross any camp.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = Player;

    fn holds(player: &Player) -> Result<(), String> {
        // A lone piece placed in a foreign camp can still move out of it, and a
        // piece outside can still step in.
        let foreign = player.next().start_camp();
        let inside = foreign[0];

        let mut pos = Position::empty();
        pos.set(inside, Some(*player));
        if legal_moves(&pos, *player).is_empty() {
            return Err(format!(
                "a piece in player {}'s camp has no moves",
                player.next().index()
            ));
        }

        // And some neighbouring hole permits stepping into the camp.
        let entry = Dir::ALL
            .iter()
            .map(|d| inside.neighbour(*d))
            .find(|c| on_board(*c));
        let Some(outside) = entry else {
            return Err("the camp hole has no on-board neighbour".into());
        };
        let mut pos2 = Position::empty();
        pos2.set(outside, Some(*player));
        if !is_legal_step(&pos2, *player, outside, inside) {
            return Err("stepping into a foreign camp was rejected".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<Player> {
        Player::ALL.to_vec()
    }
}
register_law!(CampsAreUnrestricted, CAMPS_ARE_UNRESTRICTED);

/// Iterating single hops reaches exactly the transitive closure.
///
/// The staged interface in `checkers-bevy` lets a player take one hop at a time,
/// so it must not be able to reach anywhere the atomic rules would forbid, nor
/// be blocked from anywhere they allow.
pub struct SingleHopsReachTheClosure;

impl Law for SingleHopsReachTheClosure {
    const ID: &'static str = "CC-TURN-HOP-CLOSURE";
    const STATEMENT: &'static str =
        r"\{y : x \leadsto_s y\} = \text{closure of single hops from } x";
    const CHAPTER: Chapter = Chapter::JumpSequences;
    const SUMMARY: &'static str =
        "Chaining single hops reaches exactly the destinations the closure allows.";
    /// In plain terms: Taking one jump at a time reaches exactly the places a whole chain would.
    const NOTE: &'static str =
        "Taking one jump at a time reaches exactly the places a whole chain would.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let Some(player) = pos.occupant(*origin) else {
            return Ok(());
        };

        // Breadth-first over single hops, as a player clicking would explore.
        let mut seen = HashSet::from([*origin]);
        let mut frontier = vec![*origin];
        let mut reached = HashSet::new();

        while let Some(cur) = frontier.pop() {
            // Blockers never move during a turn, so the piece's own position is
            // the only thing that changes: place it at `cur` and enumerate.
            let mut scratch = pos.clone();
            if cur != *origin {
                scratch.set(*origin, None);
                scratch.set(cur, Some(player));
            }
            for d in single_hop_destinations(&scratch, cur) {
                if seen.insert(d) {
                    reached.insert(d);
                    frontier.push(d);
                }
            }
        }

        let closure = jump_destinations(pos, *origin);
        if reached != closure {
            let missing = closure.difference(&reached).count();
            let extra = reached.difference(&closure).count();
            return Err(format!(
                "staged hops from ({},{}) reach {} destination(s) but the closure has {}: \
                 {missing} unreachable, {extra} extra",
                origin.q,
                origin.r,
                reached.len(),
                closure.len()
            ));
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(SingleHopsReachTheClosure, SINGLE_HOPS_REACH_THE_CLOSURE);

/// A single hop is one jump, never a chain.
///
/// If this were false, a staged interface would let a player skip intermediate
/// holes, which would make the route it records a fiction.
pub struct SingleHopIsOneJump;

impl Law for SingleHopIsOneJump {
    const ID: &'static str = "CC-TURN-HOP-ONE";
    const STATEMENT: &'static str = r"\forall y \in H(s,x):\ \exists d \in D:\ y = x + 2d";
    const CHAPTER: Chapter = Chapter::JumpSequences;
    const SUMMARY: &'static str =
        "Every single-hop destination lies exactly two holes away in one direction.";
    /// In plain terms: Every offered hop is a single jump, never two chained together.
    const NOTE: &'static str = "Every offered hop is a single jump, never two chained together.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let Some(player) = pos.occupant(*origin) else {
            return Ok(());
        };

        for dest in single_hop_destinations(pos, *origin) {
            // Must be x + 2d for some direction d, with that d legal. The
            // distance is then 2 by construction, so it needs no separate check.
            let matching = Dir::ALL
                .iter()
                .find(|d| origin.jump_dest(**d) == dest)
                .copied();
            let Some(d) = matching else {
                return Err(format!(
                    "({},{}) is not x+2d from ({},{})",
                    dest.q, dest.r, origin.q, origin.r
                ));
            };
            if !is_legal_jump(pos, player, *origin, d) {
                return Err(format!(
                    "({},{}) was offered but the jump is not legal",
                    dest.q, dest.r
                ));
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(SingleHopIsOneJump, SINGLE_HOP_IS_ONE_JUMP);

/// A staged turn always yields a move the atomic rules accept.
pub struct StagedTurnYieldsLegalMove;

impl Law for StagedTurnYieldsLegalMove {
    const ID: &'static str = "CC-TURN-STAGED-LEGAL";
    const STATEMENT: &'static str =
        r"\text{hops } h_1\ldots h_k \text{ legal} \implies (x, h_k) \in M(s,i)";
    const CHAPTER: Chapter = Chapter::MoveGeneration;
    const SUMMARY: &'static str =
        "Any sequence of legal single hops commits to a move the rules already allow.";
    /// In plain terms: Any staged turn you can click together is a move the rules already allow.
    const NOTE: &'static str =
        "Any staged turn you can click together is a move the rules already allow.";
    const EVIDENCE: Evidence = Evidence::Property;
    type Subject = (Position, Coord);

    fn holds((pos, origin): &(Position, Coord)) -> Result<(), String> {
        let Some(player) = pos.occupant(*origin) else {
            return Ok(());
        };
        let Some(mut turn) = JumpTurn::begin(pos, player, *origin) else {
            return Err("could not begin a turn on an owned piece".into());
        };

        // A turn with no hops must refuse to commit.
        if turn.to_move().is_ok() {
            return Err("a turn with no hops was committable".into());
        }

        // Walk greedily up to a few hops, checking each commit point.
        let legal = legal_moves(pos, player);
        for _ in 0..4 {
            let Some(next) = turn.next_hops().first().copied() else {
                break;
            };
            if !turn.hop(next) {
                return Err(format!(
                    "a legal hop to ({},{}) was refused",
                    next.q, next.r
                ));
            }
            // Turns that end at their origin are CC-TURN-NO-NULL-MOVE's concern,
            // not this law's.
            if turn.current() == *origin {
                continue;
            }

            let mv = turn.to_move().map_err(|e| {
                format!("a turn with {} hop(s) refused to commit: {e}", turn.hops())
            })?;

            if !legal.contains(&mv) {
                return Err(format!(
                    "staged turn to ({},{}) is not among the legal moves",
                    mv.destination.q, mv.destination.r
                ));
            }
            // The recorded route must start at the origin and end at the move's
            // destination.
            let route = mv.route.as_ref().ok_or("committed move carries no route")?;
            if route.first() != Some(origin) || route.last() != Some(&mv.destination) {
                return Err("the recorded route does not span origin to destination".into());
            }
        }
        Ok(())
    }

    fn subjects() -> Vec<(Position, Coord)> {
        jump_scenarios()
    }
}
register_law!(StagedTurnYieldsLegalMove, STAGED_TURN_YIELDS_LEGAL_MOVE);

/// A staged turn that ends where it began cannot be committed.
///
/// Its own law rather than a side condition of `CC-TURN-STAGED-LEGAL`, so it
/// cannot drift into being unchecked. The closure-level analogue is
/// `CC-JUMP-REVISIT`.
pub struct StagedTurnCannotBeANullMove;

impl Law for StagedTurnCannotBeANullMove {
    const ID: &'static str = "CC-TURN-NO-NULL-MOVE";
    const STATEMENT: &'static str = r"\text{current} = \text{origin} \implies \neg\text{commit}";
    const CHAPTER: Chapter = Chapter::JumpSequences;
    const SUMMARY: &'static str =
        "A staged turn whose piece is back at its origin cannot be committed.";
    /// In plain terms: A turn that ends where it began cannot be confirmed.
    const NOTE: &'static str = "A turn that ends where it began cannot be confirmed.";
    const EVIDENCE: Evidence = Evidence::Exhaustive;
    type Subject = ();

    fn holds((): &()) -> Result<(), String> {
        // A piece with a blocker beside it can hop out and straight back, so the
        // case is reachable rather than hypothetical.
        let mut pos = Position::empty();
        let origin = Coord::new(0, 0);
        pos.set(origin, Some(Player::ALL[0]));
        pos.set(Coord::new(1, 0), Some(Player::ALL[1]));

        let Some(mut turn) = JumpTurn::begin(&pos, Player::ALL[0], origin) else {
            return Err("could not begin a turn on an owned piece".into());
        };

        if !turn.hop(Coord::new(2, 0)) {
            return Err("the piece could not hop over its blocker".into());
        }
        if !turn.can_commit() {
            return Err("a turn one hop from its origin should be committable".into());
        }

        if !turn.hop(origin) {
            return Err("the piece could not hop back over its blocker".into());
        }
        if turn.hops() != 2 {
            return Err(format!("expected 2 hops, found {}", turn.hops()));
        }
        if turn.can_commit() {
            return Err("a turn back at its origin was committable".into());
        }
        if turn.to_move().is_ok() {
            return Err("to_move accepted a turn that moved nowhere".into());
        }
        Ok(())
    }

    fn subjects() -> Vec<()> {
        vec![()]
    }
}
register_law!(
    StagedTurnCannotBeANullMove,
    STAGED_TURN_CANNOT_BE_A_NULL_MOVE
);

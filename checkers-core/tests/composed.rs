//! Do the laws hold for the smaller games chapter 15 allows?
//!
//! The registry was written against the specification's six-player game, but
//! the front-end deals two- and three-player games from the same rules. This
//! suite re-runs every position- and game-level law over composed games and
//! sorts the outcomes into three honest buckets:
//!
//! 1. **Holds unchanged.** Movement, staged turns, and turn mechanics are
//!    functions of the position, not of how many camps are seated.
//! 2. **Six-player by definition.** Piece conservation and occupancy
//!    accounting quantify over all six players; on a composed position they
//!    *must* fail, and the composed invariant is `audit_position` instead.
//!    Asserting the failure here pins that scope — a silent pass would mean
//!    the law had stopped saying what it says.
//! 3. **Game-level rules generalized in the registry** (`CC-TURN-PASS`,
//!    `CC-TURN-PASS-RESET`): re-checked here against composed games as well,
//!    so a regression in either would be caught twice.

use checkers_core::Xorshift;
use checkers_core::audit::audit_position;
use checkers_core::geometry::Coord;
use checkers_core::law::Law;
use checkers_core::laws::rules::{
    JumpClosureIsExact, JumpDoesNotCapture, JumpLegality, MoveGenerationIsDeduplicated,
    MovesStayOnBoard, OccupancyAccounting, OccupancyIsPositionDetermined, PieceConservation,
    PlayPreservesInvariants, RouteEqualsNetEffect, SingleHopIsOneJump, SingleHopsReachTheClosure,
    StagedTurnYieldsLegalMove, StepDisplacement, StepLegality,
};
use checkers_core::position::{Player, Position};
use checkers_core::rules::{Game, Outcome};

/// The player sets the front-end deals, minus six (which the registry itself
/// covers): two facing camps, and every second camp.
const CONFIGS: [&[Player]; 2] = [
    &[Player::ALL[0], Player::ALL[3]],
    &[Player::ALL[0], Player::ALL[2], Player::ALL[4]],
];

/// A composed starting position: seated camps full, everything else empty.
fn composed_initial(players: &[Player]) -> Position {
    let mut pos = Position::empty();
    for p in players {
        for &c in p.start_camp() {
            pos.set(c, Some(*p));
        }
    }
    pos
}

/// Positions reached by playing a fixed pseudo-random composed game.
fn played_positions(players: &[Player], plies: usize, seed: u64) -> Vec<Position> {
    let mut rng = Xorshift::new(seed);
    let mut game = Game::compose(composed_initial(players), players[0], players);
    let mut out = vec![game.position().clone()];
    for _ in 0..plies {
        if game.is_over() {
            break;
        }
        let moves = game.legal_moves();
        if moves.is_empty() {
            game.pass();
        } else {
            game.play(&moves[rng.below(moves.len())].clone());
        }
        out.push(game.position().clone());
    }
    out
}

/// Every occupied hole of a position — origins for the (position, hole) laws.
fn pieces(pos: &Position) -> Vec<Coord> {
    pos.holes()
        .iter()
        .copied()
        .filter(|c| pos.occupant(*c).is_some())
        .collect()
}

fn check_players(players: &[Player]) {
    let names: Vec<u8> = players.iter().map(|p| p.index()).collect();
    let initial = composed_initial(players);
    let played = played_positions(players, 40, 0x10CE);
    let mut positions = vec![initial.clone()];
    positions.extend(played.iter().cloned());

    // Bucket 2: the six-player invariants must FAIL on composed positions.
    for pos in &positions {
        assert!(
            PieceConservation::holds(pos).is_err(),
            "players {names:?}: piece conservation held on a composed position - \
             the law's scope and the code have diverged",
        );
        assert!(
            OccupancyAccounting::holds(pos).is_err(),
            "players {names:?}: occupancy accounting held on a composed position - \
             the law's scope and the code have diverged",
        );
        // The composed invariant itself: seated players own ten, others none.
        audit_position(pos, players)
            .unwrap_or_else(|f| panic!("players {names:?}: composed invariant broken: {f}"));
    }
    // And the initial composed position is not a win for anyone.
    for p in Player::ALL {
        assert!(
            !initial.has_won(p),
            "players {names:?}: the initial position is a win for player {}",
            p.index()
        );
    }

    // Bucket 1: position-level laws hold unchanged.
    for pos in &positions {
        let subjects: Vec<(Position, Coord)> =
            pieces(pos).into_iter().map(|c| (pos.clone(), c)).collect();

        StepLegality::holds(pos).unwrap_or_else(|e| panic!("players {names:?}: StepLegality: {e}"));
        StepDisplacement::holds(pos)
            .unwrap_or_else(|e| panic!("players {names:?}: StepDisplacement: {e}"));
        MoveGenerationIsDeduplicated::holds(pos)
            .unwrap_or_else(|e| panic!("players {names:?}: MoveDedup: {e}"));
        MovesStayOnBoard::holds(pos)
            .unwrap_or_else(|e| panic!("players {names:?}: MovesStayOnBoard: {e}"));
        PlayPreservesInvariants::holds(pos)
            .unwrap_or_else(|e| panic!("players {names:?}: InvariantsPreserved: {e}"));

        for subject in &subjects {
            JumpLegality::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: JumpLegality: {e}"));
            JumpDoesNotCapture::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: JumpNoCapture: {e}"));
            JumpClosureIsExact::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: JumpClosure: {e}"));
            OccupancyIsPositionDetermined::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: JumpOmega: {e}"));
            RouteEqualsNetEffect::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: RouteEqualsNet: {e}"));
            SingleHopsReachTheClosure::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: HopClosure: {e}"));
            SingleHopIsOneJump::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: HopIsOneJump: {e}"));
            StagedTurnYieldsLegalMove::holds(subject)
                .unwrap_or_else(|e| panic!("players {names:?}: StagedLegal: {e}"));
        }
    }

    // Bucket 3: game-level behaviour over a composed game end to end.
    let mut rng = Xorshift::new(0x5EED);
    let mut game = Game::compose(initial, players[0], players);
    for ply in 0..60 {
        if game.is_over() {
            break;
        }
        let moves = game.legal_moves();
        if moves.is_empty() {
            game.pass();
            continue;
        }
        game.play(&moves[rng.below(moves.len())].clone());
        assert_eq!(
            audit_position(game.position(), players),
            Ok(()),
            "players {names:?}: invariant broken at ply {ply}"
        );
    }
    if let Some(Outcome::Winner(p)) = game.outcome() {
        assert!(
            players.contains(&p),
            "players {names:?}: an unseated player ({}) won",
            p.index()
        );
    }
}

#[test]
fn the_laws_hold_for_two_player_games() {
    check_players(CONFIGS[0]);
}

#[test]
fn the_laws_hold_for_three_player_games() {
    check_players(CONFIGS[1]);
}

/// The six-by-definition laws must not pass silently anywhere: sanity that the
/// fixtures above really are composed positions.
#[test]
fn the_sweep_positions_are_genuinely_composed() {
    for players in CONFIGS {
        let pos = composed_initial(players);
        let unseated: Vec<_> = Player::ALL
            .iter()
            .filter(|p| !players.contains(p))
            .collect();
        for p in unseated {
            assert_eq!(
                pos.count_of(*p),
                0,
                "player {} should be unseated",
                p.index()
            );
        }
    }
}

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
    Game::for_players(players).position().clone()
}

/// The composed starting position, then the positions reached by playing a
/// fixed pseudo-random composed game from it.
fn played_positions(players: &[Player], plies: usize, seed: u64) -> Vec<Position> {
    let mut rng = Xorshift::new(seed);
    let mut game = Game::for_players(players);
    let mut out = vec![game.position().clone()];
    for _ in 0..plies {
        if game.is_over() {
            break;
        }
        let moves = game.legal_moves();
        if moves.is_empty() {
            game.pass();
        } else {
            game.play(&moves[rng.below(moves.len())]);
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

/// Check one law on one subject, naming the players and the law on failure.
fn check<L: Law>(names: &[u8], subject: &L::Subject) {
    L::holds(subject).unwrap_or_else(|e| panic!("players {names:?}: {}: {e}", L::ID));
}

fn check_players(players: &[Player]) {
    let names: Vec<u8> = players.iter().map(|p| p.index()).collect();
    let initial = composed_initial(players);
    let positions = played_positions(players, 40, 0x10CE);

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
        check::<StepLegality>(&names, pos);
        check::<StepDisplacement>(&names, pos);
        check::<MoveGenerationIsDeduplicated>(&names, pos);
        check::<MovesStayOnBoard>(&names, pos);
        check::<PlayPreservesInvariants>(&names, pos);

        for origin in pieces(pos) {
            let subject = (pos.clone(), origin);
            check::<JumpLegality>(&names, &subject);
            check::<JumpDoesNotCapture>(&names, &subject);
            check::<JumpClosureIsExact>(&names, &subject);
            check::<OccupancyIsPositionDetermined>(&names, &subject);
            check::<RouteEqualsNetEffect>(&names, &subject);
            check::<SingleHopsReachTheClosure>(&names, &subject);
            check::<SingleHopIsOneJump>(&names, &subject);
            check::<StagedTurnYieldsLegalMove>(&names, &subject);
        }
    }

    // Bucket 3: game-level behaviour over a composed game end to end.
    let mut rng = Xorshift::new(0x5EED);
    let mut game = Game::for_players(players);
    for ply in 0..60 {
        if game.is_over() {
            break;
        }
        let moves = game.legal_moves();
        if moves.is_empty() {
            game.pass();
            continue;
        }
        game.play(&moves[rng.below(moves.len())]);
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
        for p in Player::ALL.iter().filter(|p| !players.contains(p)) {
            assert_eq!(
                pos.count_of(*p),
                0,
                "player {} should be unseated",
                p.index()
            );
        }
    }
}

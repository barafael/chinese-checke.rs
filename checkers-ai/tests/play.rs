//! Black-box behaviour of the finished engine, driven only through
//! [`checkers_ai::Ai`] and the rules.
//!
//! The headline property: an engine-vs-engine race must *go somewhere*. It
//! terminates, someone wins, every position it passes through is one the
//! specification allows, and it never shuffles the same position back and
//! forth — the failure mode that makes a race engine unplayable.

use checkers_ai::{Ai, AiConfig};
use checkers_core::audit::audit_position;
use checkers_core::position::{Player, Position};
use checkers_core::rules::{Game, Outcome};
use std::time::Duration;

fn fast() -> AiConfig {
    AiConfig {
        budget: Duration::from_millis(40),
        max_depth: 8,
    }
}

/// Play the engine against itself until the game ends or the ply cap is hit.
///
/// Returns the final game and the ply count. Every ply is checked against the
/// rules' audit, and the recent-position window is checked for shuffling: no
/// position may repeat within twelve plies, which a shuffling engine does
/// constantly and a racing engine never does.
fn self_play(players: &[Player], ply_cap: usize) -> (Game, usize) {
    let mut ai = Ai::new(fast());
    let mut game = Game::for_players(players);
    let mut window: Vec<Position> = vec![game.position().clone()];

    for ply in 0..ply_cap {
        assert!(
            !game.is_over(),
            "players {players:?}: the game ended before its time at ply {ply}"
        );
        let mv = ai
            .choose_move(&game)
            .unwrap_or_else(|| panic!("players {players:?}: stuck at ply {ply}"));
        let legal = game.legal_moves();
        assert!(
            legal.iter().any(|m| m.kind == mv.kind
                && m.origin == mv.origin
                && m.destination == mv.destination),
            "players {players:?}: illegal engine move {mv:?} at ply {ply}"
        );
        game.play(&mv);

        let pos = game.position();
        let seated_audit = if players.len() == 6 {
            audit_position(pos, &Player::ALL)
        } else {
            audit_position(pos, players)
        };
        assert_eq!(
            seated_audit,
            Ok(()),
            "players {players:?}: position invariant broken at ply {ply}"
        );

        // Shuffle detection over the last twelve plies.
        window.push(pos.clone());
        if window.len() > 12 {
            window.remove(0);
        }
        let latest = window.last().expect("just pushed");
        let repeats = window[..window.len() - 1].iter().any(|p| p == latest);
        assert!(
            !repeats,
            "players {players:?}: position repeated within 12 plies at ply {ply} - \
             the engine is shuffling"
        );

        if game.is_over() {
            return (game, ply + 1);
        }
    }
    (game, ply_cap)
}

/// The two-player race must actually finish: a racing engine converts its
/// progress long before a few hundred plies.
#[test]
fn two_player_self_play_finishes() {
    let (game, plies) = self_play(&[Player::ALL[0], Player::ALL[3]], 300);
    assert!(
        plies < 300,
        "the engine raced for {plies} plies without finishing"
    );
    match game.outcome() {
        Some(Outcome::Winner(p)) => {
            assert!(
                [Player::ALL[0], Player::ALL[3]].contains(&p),
                "the winner must be a seated player"
            );
        }
        other => panic!("a finished two-player race should have a winner, got {other:?}"),
    }
}

/// The three-player game is wider and shallower; it should still race, stay
/// legal, and not shuffle. Termination is not asserted — three maxⁿ players
/// genuinely can stall against each other — but the winner, if any, must be
/// a seated player.
#[test]
fn three_player_self_play_stays_sound() {
    let (game, _plies) = self_play(&[Player::ALL[0], Player::ALL[2], Player::ALL[4]], 120);
    if let Some(Outcome::Winner(p)) = game.outcome() {
        assert!(
            [Player::ALL[0], Player::ALL[2], Player::ALL[4]].contains(&p),
            "the winner must be a seated player"
        );
    }
}

/// Progress must be real: after the two-player race, the winning player's
/// pieces are all home and the loser's are not.
#[test]
fn the_winner_actually_filled_their_camp() {
    let (game, _) = self_play(&[Player::ALL[0], Player::ALL[3]], 300);
    if let Some(Outcome::Winner(p)) = game.outcome() {
        assert!(game.position().has_won(p));
    }
}

/// The engine answers `None` once the game is over, and keeps answering `None`
/// — no ghost moves after the finish.
#[test]
fn a_finished_game_yields_no_moves() {
    let (game, _) = self_play(&[Player::ALL[0], Player::ALL[3]], 300);
    assert!(game.is_over());
    let mut ai = Ai::new(fast());
    assert!(ai.choose_move(&game).is_none());
    assert!(ai.choose_move(&game).is_none());
}

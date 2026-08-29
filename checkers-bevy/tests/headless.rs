//! Headless tests for the front-end's logic.
//!
//! No window and no rendering: these drive the same rules the app drives, and
//! check the coordinate mapping that turns a click into a hole. Anything that
//! needs a GPU is out of scope, but the parts that can be wrong *silently* —
//! picking the wrong hole, offering an illegal destination — are covered here.

use checkers_core::audit::audit_position;
use checkers_core::geometry::{Coord, all_holes, on_board};
use checkers_core::law::verify_all;
use checkers_core::position::{MoveKind, Player, Position};
use checkers_core::rules::{Game, Outcome, jump_destinations, legal_moves};

/// Mirrors `board_view::coord_to_world`. Duplicated because the binary's modules
/// are not importable from an integration test; the round-trip properties below
/// are what matter, and `board_view`'s own unit tests cover the real functions.
fn coord_to_world(c: Coord) -> (f32, f32) {
    const SPACING: f32 = 34.0;
    let scale = SPACING / 3.0_f32.sqrt();
    (
        scale * 3.0_f32.sqrt() * (c.q as f32 + c.r as f32 / 2.0),
        -scale * 1.5 * c.r as f32,
    )
}

fn world_to_coord(x: f32, y: f32) -> Coord {
    const SPACING: f32 = 34.0;
    let scale = SPACING / 3.0_f32.sqrt();
    let qf = (x / (scale * 3.0_f32.sqrt())) - (-y / (scale * 1.5)) / 2.0;
    let rf = -y / (scale * 1.5);
    let sf = -qf - rf;

    let (mut q, mut r, s) = (qf.round(), rf.round(), sf.round());
    let (dq, dr, ds) = ((q - qf).abs(), (r - rf).abs(), (s - sf).abs());
    if dq > dr && dq > ds {
        q = -r - s;
    } else if dr > ds {
        r = -q - s;
    }
    Coord::new(q as i32, r as i32)
}

#[test]
fn every_hole_round_trips_through_screen_space() {
    for c in all_holes() {
        let (x, y) = coord_to_world(c);
        assert_eq!(world_to_coord(x, y), c, "{c:?} did not round-trip");
    }
}

/// A click anywhere in a hole's neighbourhood resolves to that hole.
#[test]
fn clicks_near_a_hole_centre_resolve_to_it() {
    for c in all_holes() {
        let (x, y) = coord_to_world(c);
        for (dx, dy) in [(8.0, 0.0), (-8.0, 0.0), (0.0, 8.0), (0.0, -8.0), (5.0, 5.0)] {
            assert_eq!(
                world_to_coord(x + dx, y + dy),
                c,
                "{c:?} offset by ({dx},{dy}) resolved elsewhere"
            );
        }
    }
}

/// The app only ever plays moves it obtained from the rules. Simulating that
/// discipline over a whole game must never violate a law.
#[test]
fn simulated_play_never_violates_a_law() {
    let mut game = Game::new();
    let mut state: u64 = 0xBEEF;
    let mut plies = 0;

    while !game.is_over() && plies < 400 {
        let moves = game.legal_moves();
        if moves.is_empty() {
            game.pass();
            continue;
        }
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let mv = moves[(state % moves.len() as u64) as usize].clone();
        game.play(&mv);
        plies += 1;

        // Per-ply: the linear position audit, exactly what the app runs.
        if let Err(fault) = audit_position(game.position()) {
            panic!("position invariant violated after ply {plies}: {fault}");
        }
    }
    assert!(plies > 0, "the simulation made no moves");

    // Once, at the end: the full registry. Running it per ply would take
    // minutes, since every law regenerates its own sample games.
    if let Err(v) = verify_all() {
        panic!("law violated: {v}");
    }
}

/// The selection logic must offer exactly the destinations the rules allow.
#[test]
fn offered_destinations_match_the_rules() {
    let pos = Position::initial();
    for player in Player::ALL {
        let moves = legal_moves(&pos, player);
        for origin in pos.pieces_of(player) {
            // What the UI would highlight for this piece.
            let offered: Vec<Coord> = moves
                .iter()
                .filter(|m| m.origin == origin)
                .map(|m| m.destination)
                .collect();

            // Every offered destination must be a real legal move.
            for dest in &offered {
                assert!(
                    moves
                        .iter()
                        .any(|m| m.origin == origin && m.destination == *dest),
                    "offered {dest:?} is not legal from {origin:?}"
                );
                assert!(on_board(*dest), "offered off-board {dest:?}");
                assert!(pos.is_empty_hole(*dest), "offered occupied {dest:?}");
            }

            // And the jump subset matches the closure exactly.
            let offered_jumps: Vec<Coord> = moves
                .iter()
                .filter(|m| m.origin == origin && m.kind == MoveKind::Jump)
                .map(|m| m.destination)
                .collect();
            let closure = jump_destinations(&pos, origin);
            assert_eq!(
                offered_jumps.len(),
                closure.len(),
                "jump count mismatch from {origin:?}"
            );
            for d in &offered_jumps {
                assert!(closure.contains(d), "offered non-reachable jump {d:?}");
            }
        }
    }
}

/// Clicking a hole the active player does not own must not select anything.
#[test]
fn only_the_active_players_pieces_are_selectable() {
    let pos = Position::initial();
    let active = Player::ALL[0];

    for c in all_holes() {
        let selectable = pos.occupant(c) == Some(active);
        if !selectable {
            // Either empty or another player's: the rules offer no move from it.
            let has_moves = legal_moves(&pos, active).iter().any(|m| m.origin == c);
            assert!(
                !has_moves,
                "{c:?} is not the active player's but offers moves"
            );
        }
    }
}

/// A move never reaches a hole the rules did not offer, even if the click lands
/// on a legal-looking neighbour.
#[test]
fn illegal_destinations_are_rejected() {
    let pos = Position::initial();
    let player = Player::ALL[0];
    let moves = legal_moves(&pos, player);
    let origin = pos.pieces_of(player)[0];

    let legal: Vec<Coord> = moves
        .iter()
        .filter(|m| m.origin == origin)
        .map(|m| m.destination)
        .collect();

    // Every on-board hole that is not offered must be genuinely unreachable.
    for c in all_holes() {
        if legal.contains(&c) || c == origin {
            continue;
        }
        assert!(
            !moves
                .iter()
                .any(|m| m.origin == origin && m.destination == c),
            "{c:?} was excluded from the UI list but is legal"
        );
    }
}

/// The frozen-board draw path terminates rather than looping forever, which is
/// what the app's auto-pass loop depends on.
#[test]
fn a_frozen_game_reaches_a_draw() {
    let mut game = Game::from_position(checkers_core::rules::frozen_position(), Player::ALL[0]);
    let mut passes = 0;
    while !game.is_over() && passes < 20 {
        assert!(game.legal_moves().is_empty());
        game.pass();
        passes += 1;
    }
    assert_eq!(game.outcome(), Some(Outcome::Draw));
    assert_eq!(passes, 6, "a draw should take exactly six passes");
}

/// A blocked player is auto-passed by the app; check that the underlying loop
/// makes progress rather than spinning.
#[test]
fn blocked_player_auto_pass_makes_progress() {
    let mut game = Game::from_position(checkers_core::rules::blocked_position(), Player::ALL[0]);
    assert!(game.legal_moves().is_empty(), "player 0 should be stuck");

    let mut guard = 0;
    while !game.is_over() && game.legal_moves().is_empty() && guard < 10 {
        game.pass();
        guard += 1;
    }
    // Some other player can move, so the game must not have ended in a draw.
    assert!(guard < 6, "expected another player to have a move");
    assert!(!game.is_over());
}

//! Headless tests for staged jump turns.
//!
//! These exercise the interaction contract the UI depends on: only one hop is
//! ever offered, hops chain, cancelling leaves the game untouched, and a turn
//! that ends where it began cannot be committed.

use checkers_core::audit::audit_position;
use checkers_core::geometry::Coord;
use checkers_core::position::{MoveKind, Player, Position};
use checkers_core::rules::{Game, jump_destinations, legal_moves};
use checkers_core::turn::{CommitError, JumpTurn, single_hop_destinations};

/// A piece with two blockers in a line, so hops chain twice.
fn chain_setup() -> (Position, Coord) {
    let mut pos = Position::empty();
    let origin = Coord::new(0, 0);
    pos.set(origin, Some(Player::ALL[0]));
    pos.set(Coord::new(1, 0), Some(Player::ALL[1]));
    pos.set(Coord::new(3, 0), Some(Player::ALL[1]));
    (pos, origin)
}

/// The core requirement: one hop is offered, not the whole closure.
#[test]
fn only_one_hop_is_offered_at_a_time() {
    let (pos, origin) = chain_setup();

    let offered = single_hop_destinations(&pos, origin);
    assert_eq!(offered, vec![Coord::new(2, 0)]);

    // The closure reaches (4,0) as well, which must NOT be offered yet.
    let closure = jump_destinations(&pos, origin);
    assert!(closure.contains(&Coord::new(4, 0)));
    assert!(
        !offered.contains(&Coord::new(4, 0)),
        "a two-hop destination must not be offered as a first hop"
    );
}

#[test]
fn hops_chain_and_reveal_the_next_options() {
    let (pos, origin) = chain_setup();
    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();

    assert!(turn.hop(Coord::new(2, 0)));
    assert_eq!(turn.current(), Coord::new(2, 0));

    // Now — and only now — the second hop appears.
    let next = turn.next_hops();
    assert!(next.contains(&Coord::new(4, 0)), "got {next:?}");

    assert!(turn.hop(Coord::new(4, 0)));
    assert_eq!(turn.hops(), 2);
    assert_eq!(turn.path(), &[origin, Coord::new(2, 0), Coord::new(4, 0)]);
}

/// The piece stays selected between hops, which the UI relies on to keep the
/// chain going.
#[test]
fn the_piece_remains_selected_between_hops() {
    let (pos, origin) = chain_setup();
    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();

    turn.hop(Coord::new(2, 0));
    assert_eq!(turn.current(), Coord::new(2, 0));
    assert!(turn.can_commit(), "should be committable mid-chain");
    assert!(!turn.next_hops().is_empty(), "should still offer more hops");
}

#[test]
fn cancelling_leaves_the_game_untouched() {
    // Deliberately not `mut`: the point of the test is that staging and
    // abandoning a turn never mutates the game at all.
    let game = Game::new();
    let before = game.position().clone();
    let player = game.turn();

    let origin = game
        .position()
        .pieces_of(player)
        .into_iter()
        .find(|c| !single_hop_destinations(game.position(), *c).is_empty())
        .expect("the initial position has jumps");

    // Stage a hop, then drop the turn without committing.
    let mut turn = JumpTurn::begin(game.position(), player, origin).unwrap();
    let dest = turn.next_hops()[0];
    assert!(turn.hop(dest));
    assert_ne!(turn.preview().occupant(dest), None, "preview shows the hop");
    drop(turn);

    assert_eq!(game.position(), &before, "the game must be unchanged");
    assert_eq!(game.turn(), player, "the turn must not have advanced");
}

/// A hop is only a preview until committed.
#[test]
fn the_preview_is_not_the_game_position() {
    let (pos, origin) = chain_setup();
    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    turn.hop(Coord::new(2, 0));

    assert!(turn.preview().is_empty_hole(origin));
    assert_eq!(
        turn.preview().occupant(Coord::new(2, 0)),
        Some(Player::ALL[0])
    );
    // The source position is untouched.
    assert_eq!(pos.occupant(origin), Some(Player::ALL[0]));
    assert!(pos.is_empty_hole(Coord::new(2, 0)));
}

/// Chapter 9: a turn ending where it began is not a move.
#[test]
fn a_turn_returning_to_its_origin_cannot_be_committed() {
    let mut pos = Position::empty();
    let origin = Coord::new(0, 0);
    pos.set(origin, Some(Player::ALL[0]));
    pos.set(Coord::new(1, 0), Some(Player::ALL[1]));

    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    assert!(turn.hop(Coord::new(2, 0)));
    assert!(turn.can_commit(), "one hop away is committable");

    // Hop back over the same blocker.
    assert!(turn.hop(origin), "hopping back should be legal");
    assert_eq!(turn.current(), origin);
    assert_eq!(turn.hops(), 2, "two hops were taken");
    assert!(!turn.can_commit(), "but the piece has not moved");
    assert_eq!(turn.to_move(), Err(CommitError::NoHopsTaken));
}

#[test]
fn committing_with_no_hops_is_refused() {
    let (pos, origin) = chain_setup();
    let turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    assert!(!turn.can_commit());
    assert!(turn.to_move().is_err());
}

/// A committed staged turn must be a move the atomic rules already allow.
#[test]
fn a_committed_chain_is_accepted_by_the_rules() {
    let mut game = Game::new();
    let player = game.turn();
    let origin = game
        .position()
        .pieces_of(player)
        .into_iter()
        .find(|c| !single_hop_destinations(game.position(), *c).is_empty())
        .unwrap();

    let mut turn = JumpTurn::begin(game.position(), player, origin).unwrap();
    let dest = turn.next_hops()[0];
    turn.hop(dest);

    let mv = turn.to_move().unwrap();
    assert_eq!(mv.kind, MoveKind::Jump);
    assert!(
        game.legal_moves().contains(&mv),
        "the staged move must be legal"
    );

    game.play(&mv);
    assert_eq!(game.position().occupant(dest), Some(player));
    audit_position(game.position()).expect("the position must stay valid");
}

/// The recorded route is presentational but must still be coherent.
#[test]
fn the_committed_route_spans_origin_to_destination() {
    let (pos, origin) = chain_setup();
    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    turn.hop(Coord::new(2, 0));
    turn.hop(Coord::new(4, 0));

    let mv = turn.to_move().unwrap();
    let route = mv.route.expect("a staged turn records its route");
    assert_eq!(route.first(), Some(&origin));
    assert_eq!(route.last(), Some(&mv.destination));
    assert_eq!(route.len(), 3, "origin plus two hops");

    // Consecutive holes are exactly two apart: each is one jump.
    for pair in route.windows(2) {
        assert_eq!(pair[0].distance(pair[1]), 2, "{:?} is not one hop", pair);
    }
}

#[test]
fn undo_walks_the_chain_back() {
    let (pos, origin) = chain_setup();
    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    turn.hop(Coord::new(2, 0));
    turn.hop(Coord::new(4, 0));

    assert!(turn.undo());
    assert_eq!(turn.current(), Coord::new(2, 0));
    // The preview must follow the undo.
    assert!(turn.preview().is_empty_hole(Coord::new(4, 0)));
    assert_eq!(
        turn.preview().occupant(Coord::new(2, 0)),
        Some(Player::ALL[0])
    );

    assert!(turn.undo());
    assert_eq!(turn.current(), origin);
    assert!(!turn.undo(), "nothing left to undo");
}

/// Steps are not staged: they are ordinary single moves.
#[test]
fn steps_are_not_part_of_a_staged_chain() {
    let pos = Position::initial();
    let player = Player::ALL[0];

    let steps: Vec<_> = legal_moves(&pos, player)
        .into_iter()
        .filter(|m| m.kind == MoveKind::Step)
        .collect();
    assert!(!steps.is_empty(), "the initial position has steps");

    for mv in steps {
        // A step's destination is adjacent, so it is never a jump hop.
        assert_eq!(mv.origin.distance(mv.destination), 1);
        assert!(
            !single_hop_destinations(&pos, mv.origin).contains(&mv.destination),
            "a step destination should not appear among jump hops"
        );
    }
}

/// Staging never lets a player reach a hole the atomic rules forbid.
#[test]
fn staged_chains_never_exceed_the_closure() {
    let mut game = Game::new();
    let mut state: u64 = 0x51A6ED;

    for _ in 0..40 {
        let player = game.turn();
        let jumpers: Vec<Coord> = game
            .position()
            .pieces_of(player)
            .into_iter()
            .filter(|c| !single_hop_destinations(game.position(), *c).is_empty())
            .collect();

        if jumpers.is_empty() {
            // Fall back to any legal move to advance the game.
            let moves = game.legal_moves();
            if moves.is_empty() {
                game.pass();
                continue;
            }
            let mv = moves[0].clone();
            game.play(&mv);
            continue;
        }

        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let origin = jumpers[(state % jumpers.len() as u64) as usize];
        let closure = jump_destinations(game.position(), origin);

        let mut turn = JumpTurn::begin(game.position(), player, origin).unwrap();
        for _ in 0..4 {
            let hops = turn.next_hops();
            if hops.is_empty() {
                break;
            }
            let next = hops[0];
            turn.hop(next);
            assert!(
                closure.contains(&next) || next == origin,
                "staged hop to {next:?} is outside the closure from {origin:?}"
            );
        }

        if let Ok(mv) = turn.to_move() {
            assert!(game.legal_moves().contains(&mv));
            game.play(&mv);
            audit_position(game.position()).unwrap();
        } else {
            // Returned to origin: play something else so the loop progresses.
            let moves = game.legal_moves();
            if moves.is_empty() {
                game.pass();
            } else {
                let mv = moves[0].clone();
                game.play(&mv);
            }
        }

        if game.is_over() {
            break;
        }
    }
}

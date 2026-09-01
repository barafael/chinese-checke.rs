//! Tests for the interaction contract the staged-jump UI depends on.
//!
//! Deliberately narrow. `JumpTurn`'s own semantics — hops chaining, undo,
//! refusing to commit with no hops, the preview staying uncommitted — are unit
//! tested in `checkers-core/src/turn.rs`, and the closure relationship is a
//! registered law (`CC-TURN-HOP-CLOSURE`). Re-asserting those here would mean two
//! files to keep in sync for one property.
//!
//! What remains is what those do not cover: that the front-end's *sequence* of
//! calls is sound, and that the distinctions the UI draws — step vs. hop, commit
//! vs. abandon — match the rules.

use checkers_core::audit::audit_position;
use checkers_core::geometry::Coord;
use checkers_core::position::{MoveKind, Player, Position};
use checkers_core::rules::{Game, jump_destinations, legal_moves, two_hop_position};
use checkers_core::turn::{CommitError, JumpTurn, single_hop_destinations, step_destinations};

/// The premise of the whole interaction: one hop is offered, never the closure.
#[test]
fn only_one_hop_is_offered_at_a_time() {
    let (pos, origin) = two_hop_position();
    let offered = single_hop_destinations(&pos, origin);
    let far = Coord::new(4, 0);

    assert!(
        jump_destinations(&pos, origin).contains(&far),
        "the fixture should allow a two-hop chain"
    );
    assert!(
        !offered.contains(&far),
        "a two-hop destination must not be offered as a first hop"
    );
}

/// Staging then abandoning must leave the game byte-identical.
///
/// Deliberately not `mut`: the point is that no amount of staging mutates it.
#[test]
fn cancelling_leaves_the_game_untouched() {
    let game = Game::new();
    let before = game.position().clone();
    let player = game.turn();

    let origin = game
        .position()
        .pieces_of(player)
        .into_iter()
        .find(|c| !single_hop_destinations(game.position(), *c).is_empty())
        .expect("the initial position has jumps");

    let mut turn = JumpTurn::begin(game.position(), player, origin).unwrap();
    let dest = turn.next_hops()[0];
    assert!(turn.hop(dest));
    assert!(
        turn.preview().occupant(dest).is_some(),
        "the preview should show the staged hop"
    );
    drop(turn);

    assert_eq!(game.position(), &before, "the game must be unchanged");
    assert_eq!(game.turn(), player, "the turn must not have advanced");
}

/// Chapter 9: hops taken but back at the origin is not a move.
///
/// The UI shows this reason verbatim, so it needs its own variant rather than a
/// generic refusal that would misreport the case.
#[test]
fn a_turn_returning_to_its_origin_is_refused_with_its_own_reason() {
    let mut pos = Position::empty();
    let origin = Coord::new(0, 0);
    pos.set(origin, Some(Player::ALL[0]));
    pos.set(Coord::new(1, 0), Some(Player::ALL[1]));

    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    assert!(turn.hop(Coord::new(2, 0)));
    assert!(turn.can_commit(), "one hop away is committable");

    assert!(turn.hop(origin), "hopping back over the blocker is legal");
    assert_eq!(turn.hops(), 2, "two hops were taken");
    assert!(!turn.can_commit(), "but the piece has not moved");

    // Specifically not NoHopsTaken: hops *were* taken.
    assert_eq!(
        turn.to_move(),
        Err(CommitError::ReturnedToOrigin { hops: 2 })
    );
    let text = CommitError::ReturnedToOrigin { hops: 2 }.to_string();
    assert!(text.contains("back where it started"), "{text}");
}

/// A committed chain must be a move the atomic rules already accept, and must
/// leave a valid position.
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
    audit_position(game.position(), game.players()).expect("the position must stay valid");
}

/// The recorded route is presentational, but must still be coherent: it spans
/// origin to destination in single hops.
#[test]
fn the_committed_route_spans_origin_to_destination() {
    let (pos, origin) = two_hop_position();
    let mut turn = JumpTurn::begin(&pos, Player::ALL[0], origin).unwrap();
    turn.hop(Coord::new(2, 0));
    turn.hop(Coord::new(4, 0));

    let mv = turn.to_move().unwrap();
    let route = mv.route.expect("a staged turn records its route");
    assert_eq!(route.first(), Some(&origin));
    assert_eq!(route.last(), Some(&mv.destination));
    assert_eq!(route.len(), 3, "origin plus two hops");

    for pair in route.windows(2) {
        assert_eq!(pair[0].distance(pair[1]), 2, "{pair:?} is not one hop");
    }
}

/// The UI treats steps and hops differently, so the two destination sets must
/// stay disjoint: nothing is offered as both.
#[test]
fn step_and_hop_destinations_are_disjoint() {
    let pos = Position::initial();

    for player in Player::ALL {
        for origin in pos.pieces_of(player) {
            let steps = step_destinations(&pos, origin);
            let hops = single_hop_destinations(&pos, origin);

            for s in &steps {
                assert_eq!(origin.distance(*s), 1, "a step should be adjacent");
                assert!(!hops.contains(s), "{s:?} offered as both a step and a hop");
            }
            for h in &hops {
                assert_eq!(origin.distance(*h), 2, "a hop should span two holes");
            }
        }
    }
}

/// `step_destinations` must agree with `legal_moves`, since the UI now uses the
/// cheaper call instead of filtering the full move list.
#[test]
fn step_destinations_matches_legal_moves() {
    let pos = Position::initial();

    for player in Player::ALL {
        let mut expected: Vec<(Coord, Coord)> = legal_moves(&pos, player)
            .into_iter()
            .filter(|m| m.kind == MoveKind::Step)
            .map(|m| (m.origin, m.destination))
            .collect();

        let mut actual: Vec<(Coord, Coord)> = pos
            .pieces_of(player)
            .into_iter()
            .flat_map(|o| step_destinations(&pos, o).into_iter().map(move |d| (o, d)))
            .collect();

        expected.sort();
        actual.sort();
        assert_eq!(actual, expected, "player {}", player.index());
    }
}

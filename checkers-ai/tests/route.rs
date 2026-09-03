//! Jump moves must carry a playable hop route: consecutive legal hops from
//! the origin to the destination, which is what lets a viewer animate a move
//! one hop at a time.

use checkers_ai::{Ai, AiConfig};
use checkers_core::geometry::Coord;
use checkers_core::position::{Player, Position};
use checkers_core::rules::{Game, two_hop_position};
use checkers_core::turn::JumpTurn;
use std::time::Duration;

fn fast() -> AiConfig {
    AiConfig {
        budget: Duration::from_millis(40),
        max_depth: 6,
    }
}

/// In a position where the only legal moves are jumps through the two-hole
/// chain, every jump the engine picks must come with a route whose hops play
/// legally one after another — and the resulting position must be the one the
/// move promises.
#[test]
fn a_jump_move_carries_a_playable_hop_route() {
    // (0,0) holds the mover; every step neighbour is occupied; (2,0) and
    // (4,0) are the only landings, reached over (1,0) and (3,0).
    let mut pos = Position::empty();
    let origin = Coord::new(0, 0);
    pos.set(origin, Some(Player::ALL[0]));
    for c in [
        Coord::new(1, 0),
        Coord::new(1, -1),
        Coord::new(0, -1),
        Coord::new(-1, 0),
        Coord::new(0, 1),
        Coord::new(-1, 1),
        Coord::new(3, 0),
    ] {
        pos.set(c, Some(Player::ALL[3]));
    }
    let mut game = Game::compose(pos, Player::ALL[0], &[Player::ALL[0], Player::ALL[3]]);

    assert!(
        game.legal_moves()
            .iter()
            .all(|m| m.kind == checkers_core::position::MoveKind::Jump),
        "the fixture must offer only jumps"
    );

    let mut ai = Ai::new(fast());
    let Some((mv, route)) = ai.choose_move_route_for(&game, Player::ALL[0]) else {
        panic!("a moveable position must yield a move with a route");
    };
    assert_eq!(mv.kind, checkers_core::position::MoveKind::Jump);
    assert_eq!(mv.origin, origin);
    assert_eq!(
        *route.last().expect("a jump route ends at its destination"),
        mv.destination
    );

    // Play the route hop by hop through the rules' own staged turn: every hop
    // must be accepted, and the committed move must match.
    let mut turn =
        JumpTurn::begin(game.position(), Player::ALL[0], mv.origin).expect("owned piece");
    for hop in &route {
        assert!(
            turn.hop(*hop),
            "route hop to {hop:?} was refused by the rules"
        );
    }
    let committed = turn.to_move().expect("a fully played route is committable");
    assert_eq!(committed.origin, mv.origin);
    assert_eq!(committed.destination, mv.destination);
    game.play(&committed);
}

/// The classic two-rung fixture: a forward jump whose best landing is the far
/// end of the chain, whose route passes through the near rung. The route must
/// contain both rungs in order.
#[test]
fn a_chained_jump_route_passes_through_the_rungs() {
    let (pos, _origin) = two_hop_position();
    let game = Game::compose(pos, Player::ALL[0], &[Player::ALL[0], Player::ALL[3]]);
    let mut ai = Ai::new(fast());

    let Some((mv, route)) = ai.choose_move_route_for(&game, Player::ALL[0]) else {
        panic!("the fixture has moves");
    };
    if mv.destination == Coord::new(4, 0) {
        assert_eq!(
            route,
            vec![Coord::new(2, 0), Coord::new(4, 0)],
            "the far landing's route must pass through the near rung"
        );
    } else {
        assert_eq!(mv.destination, Coord::new(2, 0));
        assert_eq!(route, vec![Coord::new(2, 0)]);
    }
    // Either way the move must be one the atomic rules accept.
    assert!(
        game.legal_moves()
            .iter()
            .any(|m| m.kind == mv.kind && m.origin == mv.origin && m.destination == mv.destination)
    );
}

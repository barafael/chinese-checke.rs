//! The resignation contract.
//!
//! The core semantics — conceding ends the game, only a seated player can
//! concede, a finished game cannot end again — are unit tested in
//! `checkers-core/src/rules.rs`. What remains for the session is the sequence
//! around them: which seat gives up, what happens to the staged state, and
//! that a second resignation is inert.

use checkers_bevy::setup::Seating;
use checkers_bevy::{Selection, Session};
use checkers_core::rules::Outcome;
use checkers_core::turn::step_destinations;

/// Hotseat resignation: every seat is local, so the seat to move gives up.
#[test]
fn resigning_ends_the_round_and_clears_the_staged_state() {
    let mut session = Session::new(Seating::Two);
    let player = session.game.turn();

    // Stage a step first, so the test proves resignation sweeps the staged
    // state rather than only ending an untouched turn.
    let origin = session
        .game
        .position()
        .pieces_of(player)
        .into_iter()
        .find(|c| !step_destinations(session.game.position(), *c).is_empty())
        .expect("the initial board offers steps");
    let dest = step_destinations(session.game.position(), origin)[0];
    session.select(origin);
    session.activate(dest);
    assert!(matches!(session.selection, Selection::Pend { .. }));

    session.resign();

    assert!(session.game.is_over());
    assert_eq!(
        session.game.outcome(),
        Some(Outcome::Resigned(player)),
        "the seat to move is the one that gives up"
    );
    assert!(
        matches!(session.selection, Selection::None),
        "a resignation sweeps the staged turn"
    );
}

/// A finished game cannot be resigned again — the second call is inert, not
/// a panic and not a changed outcome.
#[test]
fn a_second_resignation_changes_nothing() {
    let mut session = Session::new(Seating::Two);
    session.resign();
    let outcome = session.game.outcome();

    session.resign();

    assert_eq!(session.game.outcome(), outcome);
}

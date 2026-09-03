//! The paced AI-vs-AI driver, driven by an injected clock — no window.
//!
//! The contract under test is what makes the demo watchable: consecutive
//! visible actions (moves, commits, individual hops) are spaced at least a
//! second apart, every committed move is one the rules offer, every position
//! passes the audit, and the race actually finishes.

use checkers_ai::{Ai, AiConfig};
use checkers_bevy::Session;
use checkers_bevy::ai::describe;
use checkers_bevy::ai::{Action, AiPace, MOVE_INTERVAL};
use checkers_bevy::setup::Seating;
use checkers_core::audit::audit_position;
use checkers_core::position::Player;
use checkers_core::rules::Outcome;
use std::time::{Duration, Instant};

#[test]
fn a_paced_demo_race_is_followable_and_legal() {
    let mut session = Session::new(Seating::Two);
    session.ai_players = vec![Player::ALL[0], Player::ALL[3]];
    let mut ai = Ai::new(AiConfig {
        budget: Duration::from_millis(40),
        max_depth: 8,
    });
    let mut pace = AiPace::default();

    let mut now = Instant::now();
    let mut moves = 0;
    let mut marked = false;
    let mut last_action: Option<Instant> = None;
    let mut log: Vec<String> = Vec::new();

    let spacing = MOVE_INTERVAL - Duration::from_millis(1);
    while !session.game.is_over() && moves < 300 {
        match pace.advance(&mut session, &mut ai, now) {
            Action::Wait => now += Duration::from_millis(100),
            Action::Hop(hole) => {
                if let Some(t) = last_action {
                    assert!(
                        now - t >= spacing,
                        "a hop came too soon after the last action"
                    );
                }
                last_action = Some(now);
                // The staged preview shows the piece sitting on the new hole.
                assert_eq!(session.selected_hole(), Some(hole));
                now += Duration::from_millis(1100);
            }
            Action::Commit(mv) | Action::Play(mv) => {
                if let Some(t) = last_action {
                    assert!(
                        now - t >= spacing,
                        "a move came too soon after the last action"
                    );
                }
                last_action = Some(now);
                let legal = session.game.legal_moves();
                assert!(
                    legal.iter().any(|m| m.kind == mv.kind
                        && m.origin == mv.origin
                        && m.destination == mv.destination),
                    "the driver committed {mv:?}, which the rules do not offer"
                );
                log.push(format!(
                    "{}. p{} {}",
                    moves + 1,
                    session.game.turn().index(),
                    describe(&mv)
                ));
                session.game.play(&mv);
                audit_position(session.game.position(), &[Player::ALL[0], Player::ALL[3]])
                    .expect("the committed position must pass the audit");
                moves += 1;
                now += Duration::from_millis(1100);
            }
            Action::Pass => {
                if let Some(t) = last_action {
                    assert!(now - t >= spacing);
                }
                last_action = Some(now);
                session.game.pass();
                moves += 1;
                now += Duration::from_millis(1100);
            }
            Action::Abandon(reason) => {
                // A genuine mutual deadlock the engine cannot resolve: an
                // honest end, not a failure — the log records it.
                log.push(format!("# {reason}"));
                marked = true;
                break;
            }
        }
    }

    // The race must resolve: a winner, or a logged deadlock. It must not run
    // past the ply cap without one of the two.
    assert!(
        session.game.is_over() || log.last().is_some_and(|l| l.starts_with('#')),
        "the demo race must finish or be abandoned inside 300 plies"
    );
    assert!(
        !session.game.is_over() || matches!(session.game.outcome(), Some(Outcome::Winner(_))),
        "a finished two-player race has a winner"
    );
    // Every committed move is logged; the only non-move line is a terminal
    // `#` abandonment marker, which must be last.
    assert_eq!(
        log.len(),
        moves + usize::from(marked),
        "every move is logged"
    );
    if marked {
        assert!(
            log.last().unwrap().starts_with('#'),
            "abandonment marker is last"
        );
    }
    assert!(moves > 0, "the race made at least one move");
    println!("{} moves, e.g. first: {:?}", log.len(), log.first());
    println!("last: {:?}", log.last());
}

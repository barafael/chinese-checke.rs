//! Differential test: the naive model against `checkers-core`.
//!
//! This is the arrangement the workspace layout promises: `checkers-model` is
//! an independent, obviously-correct implementation, and this harness plays
//! identical games on both and compares everything observable — board
//! geometry, occupancy, legal move sets, jump closures, win flags, turns, and
//! outcomes. The two crates share no code (not even the PRNG), so agreement
//! is evidence rather than tautology.
//!
//! A mismatch here means one of the two rule engines has drifted, and the
//! one with the *simpler* implementation is the likelier truth.

use std::collections::HashSet;

use checkers_core::geometry::{Coord as CoreCoord, all_holes as core_all_holes, camp_of};
use checkers_core::position::{
    Move as CoreMove, MoveKind as CoreKind, Player as CorePlayer, Position as CorePosition,
};
use checkers_core::rules::{
    Game as CoreGame, Outcome as CoreOutcome, blocked_position as core_blocked,
    jump_destinations as core_jumps, legal_moves as core_legal,
};
use checkers_model::board::{Board, Player as ModelPlayer};
use checkers_model::coord::Coord as ModelCoord;
use checkers_model::game::{
    Game as ModelGame, Outcome as ModelOutcome, blocked_position as model_blocked,
};
use checkers_model::moves::{
    Move as ModelMove, MoveKind as ModelKind, jump_destinations as model_jumps,
    legal_moves as model_legal,
};
use checkers_model::prng::Prng;
use checkers_model::state::State as ModelState;

/// The move identity both crates agree on: (is a jump, origin, destination).
type Key = (bool, (i32, i32), (i32, i32));

fn to_core(c: ModelCoord) -> CoreCoord {
    CoreCoord::new(c.q, c.r)
}

fn key_of_model(mv: &ModelMove) -> Key {
    (
        mv.kind == ModelKind::Jump,
        (mv.origin.q, mv.origin.r),
        (mv.destination.q, mv.destination.r),
    )
}

fn key_of_core(mv: &CoreMove) -> Key {
    (
        mv.kind == CoreKind::Jump,
        (mv.origin.q, mv.origin.r),
        (mv.destination.q, mv.destination.r),
    )
}

fn move_keys_model(state: &ModelState, player: ModelPlayer) -> HashSet<Key> {
    model_legal(state, player)
        .iter()
        .map(key_of_model)
        .collect()
}

fn move_keys_core(pos: &CorePosition, player: CorePlayer) -> HashSet<Key> {
    core_legal(pos, player).iter().map(key_of_core).collect()
}

/// The model board's holes, in the same sorted order `core_all_holes` yields.
fn model_holes() -> Vec<ModelCoord> {
    let mut v: Vec<ModelCoord> = Board::new().holes().collect();
    v.sort();
    v
}

/// Occupancy must agree hole by hole.
fn assert_positions_agree(pos: &CorePosition, state: &ModelState, holes: &[ModelCoord], ctx: &str) {
    for &m in holes {
        let core_owner = pos.occupant(to_core(m)).map(|p| p.index());
        assert_eq!(
            core_owner,
            state.owner(m),
            "{ctx}: occupancy differs at {m:?}"
        );
    }
}

fn assert_outcomes_agree(core: &CoreGame, model: &ModelGame, ctx: &str) {
    assert_eq!(core.is_over(), model.is_over(), "{ctx}: over/under differs");
    match (core.outcome(), model.outcome()) {
        (None, None) => {}
        (Some(CoreOutcome::Winner(p)), Some(ModelOutcome::Winner(q))) => {
            assert_eq!(p.index(), q, "{ctx}: different winner");
        }
        (Some(CoreOutcome::Draw), Some(ModelOutcome::Draw)) => {}
        other => panic!("{ctx}: outcomes diverge: {other:?}"),
    }
}

/// Play several complete games in lockstep on both engines, comparing every
/// observable after every ply.
#[test]
fn parallel_games_never_disagree() {
    let holes = model_holes();
    let mut rng = Prng::new(0xD1FF);

    for game in 0..6 {
        let mut core = CoreGame::new();
        let mut model = ModelGame::new();

        for ply in 0..150 {
            let ctx = format!("game {game} ply {ply}");
            assert_eq!(
                core.turn().index(),
                model.turn(),
                "{ctx}: turn order diverged"
            );
            assert_positions_agree(core.position(), model.state(), &holes, &ctx);

            // Every player's legal move set must be identical, not just the
            // active player's.
            for i in 0..6u8 {
                let core_set = move_keys_core(core.position(), CorePlayer::wrapping(i));
                let model_set = move_keys_model(model.state(), i);
                assert_eq!(
                    core_set, model_set,
                    "{ctx}: move sets diverge for player {i}"
                );
            }

            // Win flags agree for everyone.
            for i in 0..6u8 {
                assert_eq!(
                    core.position().has_won(CorePlayer::wrapping(i)),
                    model.state().has_won(i),
                    "{ctx}: win flag differs for player {i}"
                );
            }

            assert_outcomes_agree(&core, &model, &ctx);
            if core.is_over() {
                break;
            }

            // Choose a move from the (already compared) common set. Sorting
            // keeps a failure reproducible: same seed, same choices.
            let mut moves: Vec<Key> = move_keys_core(core.position(), core.turn())
                .into_iter()
                .collect();
            moves.sort();
            if moves.is_empty() {
                core.pass();
                model.pass();
                continue;
            }
            let (jump, (oq, or), (dq, dr)) = moves[rng.below(moves.len() as u32) as usize];

            let origin = CoreCoord::new(oq, or);
            let destination = CoreCoord::new(dq, dr);
            let core_mv = if jump {
                CoreMove::jump(origin, destination)
            } else {
                CoreMove::step(origin, destination)
            };
            let m_origin = ModelCoord::new(oq, or);
            let m_destination = ModelCoord::new(dq, dr);
            let model_mv = if jump {
                ModelMove::jump(m_origin, m_destination)
            } else {
                ModelMove::step(m_origin, m_destination)
            };

            core.play(&core_mv);
            model.play(&model_mv);
        }

        assert_outcomes_agree(&core, &model, &format!("game {game} end"));
    }
}

/// Jump reachability must agree from every occupied hole of scattered random
/// positions — the same scenarios the core's own closure law uses, but
/// answered by an independent implementation.
#[test]
fn jump_closures_agree_everywhere() {
    let holes = model_holes();
    let mut rng = Prng::new(0xC10D5);

    for round in 0..40 {
        let mut core_pos = CorePosition::empty();
        let mut model_state = ModelState::empty(Board::new());
        let mut occupied = 0usize;

        let n = 8 + rng.below(40);
        for _ in 0..n {
            let c = holes[rng.below(holes.len() as u32) as usize];
            if core_pos.is_empty_hole(to_core(c)) {
                let owner = (occupied % 6) as u8;
                model_state.set(c, Some(owner));
                core_pos.set(to_core(c), Some(CorePlayer::wrapping(owner)));
                occupied += 1;
            }
        }
        if occupied == 0 {
            continue;
        }

        for &c in &holes {
            if model_state.owner(c).is_none() {
                continue;
            }
            let core_set: HashSet<(i32, i32)> = core_jumps(&core_pos, to_core(c))
                .into_iter()
                .map(|x| (x.q, x.r))
                .collect();
            let model_set: HashSet<(i32, i32)> = model_jumps(&model_state, c)
                .into_iter()
                .map(|x| (x.q, x.r))
                .collect();
            assert_eq!(
                core_set, model_set,
                "round {round}: jump closure differs from {c:?}"
            );
        }
    }
}

/// The two boards must be the same star, hole for hole, camp for camp.
#[test]
fn board_geometry_agrees() {
    let board = Board::new();

    let core_holes: HashSet<(i32, i32)> =
        core_all_holes().into_iter().map(|c| (c.q, c.r)).collect();
    let model_holes: HashSet<(i32, i32)> = board.holes().map(|c| (c.q, c.r)).collect();
    assert_eq!(
        core_holes, model_holes,
        "the two crates build different stars"
    );

    for c in core_all_holes() {
        let core_camp = camp_of(c).map(|i| i as u8);
        let model_camp = (0..6u8).find(|&i| board.camp(i).contains(&ModelCoord::new(c.q, c.r)));
        assert_eq!(core_camp, model_camp, "camp membership differs at {c:?}");
    }
}

/// The blocked-position fixture was written twice, once per crate; the two
/// constructions must still describe the same board and the same stuck
/// players.
#[test]
fn blocked_positions_agree() {
    let core_pos = core_blocked();
    let model_state = model_blocked();
    let holes = model_holes();

    assert_positions_agree(&core_pos, &model_state, &holes, "blocked fixture");

    for i in 0..6u8 {
        let core_stuck = core_legal(&core_pos, CorePlayer::wrapping(i)).is_empty();
        let model_stuck = model_legal(&model_state, i).is_empty();
        assert_eq!(core_stuck, model_stuck, "player {i}: blocked/free differs");
    }
    // The fixture only guarantees the first player is sealed in; the blocker
    // itself still has moves, and both engines must say so alike.
    assert!(
        core_legal(&core_pos, CorePlayer::ALL[0]).is_empty(),
        "player 0 should be blocked in both engines"
    );
}

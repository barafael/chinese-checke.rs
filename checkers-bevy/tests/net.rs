//! The networking contract: sequencing is the only path into the game.
//!
//! These drive `checkers-net`'s pure parts against the real rules. The socket
//! itself is out of scope — WebRTC needs peers and a signaling server — but the
//! properties that could break *silently* are here: a move must not be applied
//! twice, must not be applied unsequenced, and must be rejected by the rules
//! rather than trusted.

use checkers_core::position::{MoveKind, Player, Position};
use checkers_core::rules::{Game, legal_moves};
use checkers_net::{NetState, WireMove};

/// Simulates the host's sequencing arm without a socket: resolve against the
/// rules, assign a seq, apply once.
fn sequence_and_apply(net: &mut NetState, game: &mut Game, wire: WireMove) -> bool {
    if wire.resolve(&game.legal_moves()).is_none() {
        return false;
    }
    let seq = net.next_seq;
    net.next_seq += 1;
    apply(net, game, seq, wire)
}

fn apply(net: &mut NetState, game: &mut Game, seq: u32, wire: WireMove) -> bool {
    if net.is_duplicate(seq) {
        return false;
    }
    let Some(mv) = wire.resolve(&game.legal_moves()) else {
        return false;
    };
    game.play(&mv);
    net.last_applied_seq = Some(seq);
    true
}

/// The property the whole design rests on: a move reaches the game only via a
/// sequence number, and the rules get the final say on legality.
#[test]
fn sequencing_is_the_only_path_to_the_game() {
    let mut net = NetState::default();
    let mut game = Game::new();

    let player = game.turn();
    let mv = game.legal_moves().first().cloned().expect("moves exist");
    let wire = WireMove::from_move(&mv);

    assert!(sequence_and_apply(&mut net, &mut game, wire));
    assert_eq!(net.last_applied_seq, Some(0));
    assert_eq!(net.next_seq, 1);
    assert_eq!(game.position().occupant(mv.destination), Some(player));
}

/// A `Sequenced` message redelivered must not advance the game twice. Without
/// this the board would drift from every other peer's.
#[test]
fn a_duplicate_sequence_number_is_ignored() {
    let mut net = NetState::default();
    let mut game = Game::new();

    let mv = game.legal_moves().first().cloned().unwrap();
    let wire = WireMove::from_move(&mv);
    assert!(apply(&mut net, &mut game, 0, wire));

    let after = game.position().clone();
    assert!(
        !apply(&mut net, &mut game, 0, wire),
        "same seq must be dropped"
    );
    assert_eq!(game.position(), &after, "the position must not move");
    assert_eq!(
        game.turn(),
        Player::ALL[1],
        "nor may the turn advance twice"
    );
}

/// A peer sending a structurally valid but illegal move must not affect the
/// game, and must not consume a sequence number — a gap would be
/// indistinguishable from a dropped message.
#[test]
fn an_illegal_move_is_rejected_without_consuming_a_sequence_number() {
    let mut net = NetState::default();
    let mut game = Game::new();
    let before = game.position().clone();

    let bogus = WireMove {
        origin: (0, 0),
        destination: (0, 1),
        jump: false,
    };
    assert!(!sequence_and_apply(&mut net, &mut game, bogus));
    assert_eq!(net.next_seq, 0, "a rejected move must not burn a seq");
    assert_eq!(net.last_applied_seq, None);
    assert_eq!(game.position(), &before);
}

/// A move that was legal when submitted but is not by the time it is sequenced
/// (another player got there first) must be dropped, not forced.
#[test]
fn a_move_made_stale_by_reordering_is_dropped() {
    let mut net = NetState::default();
    let mut game = Game::new();

    let player = game.turn();
    let mine = game.legal_moves().first().cloned().unwrap();
    let wire = WireMove::from_move(&mine);

    // Someone else's move lands first, so it is no longer our turn.
    assert!(sequence_and_apply(&mut net, &mut game, wire));
    assert_ne!(game.turn(), player, "the turn advanced");

    // Resubmitting the same move now refers to a piece that already moved.
    let stale = sequence_and_apply(&mut net, &mut game, wire);
    assert!(!stale, "a stale move must be refused by the rules");
    assert_eq!(net.next_seq, 1, "and must not burn a seq");
}

/// Every legal move survives the wire round-trip. If any did not, that move
/// would be unplayable online while working offline.
#[test]
fn every_legal_move_survives_the_wire() {
    let pos = Position::initial();
    for player in Player::ALL {
        let legal = legal_moves(&pos, player);
        assert!(!legal.is_empty(), "player {} has moves", player.index());

        for mv in &legal {
            let wire = WireMove::from_move(mv);
            let back = wire
                .resolve(&legal)
                .expect("a legal move must resolve back");
            assert_eq!(back.origin, mv.origin);
            assert_eq!(back.destination, mv.destination);
            assert_eq!(back.kind, mv.kind);
        }
    }
}

/// Chapter 10 identity: a step and a jump to the same hole are different moves,
/// so the wire form must keep them apart.
#[test]
fn the_wire_form_preserves_move_identity() {
    let pos = Position::initial();
    let legal = legal_moves(&pos, Player::ALL[0]);

    let jump = legal
        .iter()
        .find(|m| m.kind == MoveKind::Jump)
        .expect("the initial position has jumps");
    let wire = WireMove::from_move(jump);
    assert!(wire.jump);

    // Flipping only the kind must no longer resolve to that move.
    let as_step = WireMove {
        jump: false,
        ..wire
    };
    match as_step.resolve(&legal) {
        None => {}
        Some(m) => assert_eq!(
            m.kind,
            MoveKind::Step,
            "a step-flagged wire move must never resolve to a jump"
        ),
    }
}

/// The host's seating must reach every guest.
///
/// Before `NetMsg::Start` carried `players`, a guest built its board from its
/// own local default: a host starting a three-player game left the guest
/// playing six, each peer convinced it was right. Nothing detected it, because
/// both boards were individually valid — they simply were not the same board.
#[test]
fn the_hosts_seating_reaches_the_guest_over_the_wire() {
    use checkers_bevy::setup::Seating;
    use checkers_net::{NetMsg, Seat, decode, encode};

    for host_seating in Seating::ALL {
        let sent = NetMsg::Start {
            seats: vec![Seat {
                peer: "host".into(),
                name: "host".into(),
                player: Some(0),
                ready: true,
                spectate: false,
            }],
            players: host_seating.indices(),
        };

        let bytes = encode(&sent).expect("Start must encode");
        let received = decode(&bytes).expect("Start must decode");

        let NetMsg::Start { players, .. } = received else {
            panic!("decoded to the wrong variant");
        };

        assert_eq!(
            Seating::from_indices(&players),
            Some(host_seating),
            "{host_seating:?} did not survive the wire"
        );

        // And the board the guest deals must be the host's board, hole for hole.
        let guest_board = Seating::from_indices(&players)
            .expect("just checked")
            .position();
        assert_eq!(
            guest_board,
            host_seating.position(),
            "{host_seating:?}: guest dealt a different board"
        );
    }
}

/// A guest must not deal a board it does not understand.
///
/// `from_indices` returns `None` rather than guessing, so the caller can say so.
/// Rounding an unknown seating to the nearest known one would put two peers on
/// different boards while both believed they agreed.
#[test]
fn an_unknown_seating_from_the_host_is_not_guessed() {
    use checkers_bevy::setup::Seating;
    use checkers_net::{NetMsg, decode, encode};

    // Four players: sound and playable, but not a seating this build offers.
    let sent = NetMsg::Start {
        seats: Vec::new(),
        players: vec![0, 1, 2, 3],
    };
    let bytes = encode(&sent).expect("must encode");
    let NetMsg::Start { players, .. } = decode(&bytes).expect("must decode") else {
        panic!("wrong variant");
    };
    assert_eq!(
        Seating::from_indices(&players),
        None,
        "an unoffered seating must not be silently accepted"
    );
}

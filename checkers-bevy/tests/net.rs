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

/// The host's roster must reach every guest.
///
/// Guests take `Start`'s `seats` verbatim and derive the board from the
/// claimed corners. Before `NetMsg::Start` carried `seats`, a guest built its
/// board from its own local default: a host starting a three-player game left
/// the guest playing six, each peer convinced it was right. Nothing detected
/// it, because both boards were individually valid — they simply were not the
/// same board.
#[test]
fn the_hosts_roster_reaches_the_guest_over_the_wire() {
    use checkers_net::{NetMsg, Seat, decode, encode};

    let net = NetState {
        seats: vec![
            Seat {
                peer: "host".into(),
                name: "host".into(),
                player: Some(0),
                engine: false,
            },
            Seat {
                peer: "guest".into(),
                name: "grace".into(),
                player: Some(3),
                engine: false,
            },
        ],
        ..Default::default()
    };
    let sent = checkers_bevy::lobby::start_message(&net, Default::default());
    let NetMsg::Start { seats, .. } = &sent else {
        panic!("start_message must build a Start");
    };

    let bytes = encode(&sent).expect("Start must encode");
    let NetMsg::Start { seats: back, .. } = decode(&bytes).expect("Start must decode") else {
        panic!("decoded to the wrong variant");
    };
    assert_eq!(&back, seats, "the roster must arrive intact");
}

/// A guest must not deal a board it does not understand, but it also must not
/// refuse a sound one merely because it is not a preset: corners are arbitrary
/// now, so the claimed camps are the board, whatever subset they form.
#[test]
fn arbitrary_claimed_corners_reach_the_guests_board() {
    use checkers_net::{NetMsg, decode, encode};

    // Camps 0, 1 and 4: playable, and not one of the 2/3/6 presets.
    let players = [0u32, 1, 4];
    let sent = NetMsg::Start {
        seats: Vec::new(),
        forbid_foreign_camps: false,
    };
    let bytes = encode(&sent).expect("must encode");
    let NetMsg::Start { .. } = decode(&bytes).expect("must decode") else {
        panic!("wrong variant");
    };

    let camps: Vec<Player> = players
        .iter()
        .filter_map(|&i| Player::new(i as u8))
        .collect();
    let game = Game::for_players(&camps);
    for player in Player::ALL {
        let found = game.position().pieces_of(player).len();
        assert_eq!(
            found,
            if camps.contains(&player) { 10 } else { 0 },
            "player {} must be seated iff claimed",
            player.index()
        );
    }
}

//! In-game networking: submit local moves, apply sequenced ones.
//!
//! The one rule: **a move is applied only when it arrives sequenced**. Local
//! moves go into the outbox and come back through the same path as remote
//! ones, so every peer sees one ordering. Solo play takes the same path — the
//! lone peer is its own sequencer — so the networked code is always
//! exercised.

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_net::{CH_RELIABLE, NetMsg, NetState, WireMove, broadcast, decode, send_to};

use crate::{Session, audit};

/// Drain the outbox, then apply whatever arrived.
pub fn pump(
    socket: Option<ResMut<MatchboxSocket>>,
    mut net: ResMut<NetState>,
    mut session: ResMut<Session>,
) {
    let Some(mut socket) = socket else {
        // No socket at all: apply locally so the game is still playable. Only
        // reachable if the lobby never opened one.
        apply_outbox_directly(&mut session);
        return;
    };

    for (peer, state) in socket.update_peers() {
        match state {
            PeerState::Connected => {
                if !net.peers.contains(&peer) {
                    net.peers.push(peer);
                }
            }
            PeerState::Disconnected => net.peers.retain(|p| *p != peer),
        }
    }

    let peers = net.peers.clone();
    let host = peers
        .iter()
        .copied()
        .min_by_key(|p| p.to_string())
        .filter(|_| !net.sequences());

    // 1. Submit local moves. The sequencer handles its own immediately, through
    //    the same arm that handles guests' — one serialization point.
    for mv in std::mem::take(&mut session.outbox) {
        let wire = WireMove::from_move(&mv);
        if net.sequences() {
            sequence_and_broadcast(&mut socket, &mut net, &mut session, &peers, wire);
        } else if let Some(host) = host {
            send_to(&mut socket, host, &NetMsg::Move(wire));
        } else {
            warn!("no host to submit to; dropping the move");
        }
    }

    // 2. Apply what arrived.
    let inbox: Vec<(PeerId, Box<[u8]>)> = socket.channel_mut(CH_RELIABLE).receive();
    for (_from, raw) in inbox {
        let Some(msg) = decode(&raw) else {
            continue;
        };
        match msg {
            // A guest's submission: order it and rebroadcast.
            NetMsg::Move(wire) if net.sequences() => {
                sequence_and_broadcast(&mut socket, &mut net, &mut session, &peers, wire);
            }
            NetMsg::Sequenced { seq, mv } => apply(&mut net, &mut session, seq, mv),
            // A guest cannot sequence, and lobby traffic is over.
            NetMsg::Move(_)
            | NetMsg::Hello { .. }
            | NetMsg::Roster(_)
            | NetMsg::Ready(_)
            | NetMsg::Start { .. } => {}
        }
    }
}

/// Assign the next sequence number, tell everyone, and apply locally.
fn sequence_and_broadcast(
    socket: &mut MatchboxSocket,
    net: &mut NetState,
    session: &mut Session,
    peers: &[PeerId],
    wire: WireMove,
) {
    // Reject before spending a sequence number: an illegal move must not
    // consume one, or peers would see a gap and could not tell a dropped
    // message from a rejected one.
    if wire.resolve(&session.game.legal_moves()).is_none() {
        warn!(?wire, "refusing to sequence a move the rules reject");
        return;
    }

    let seq = net.next_seq;
    net.next_seq += 1;
    broadcast(socket, peers, &NetMsg::Sequenced { seq, mv: wire });
    apply(net, session, seq, wire);
}

/// Apply a sequenced move, if it is new and legal.
fn apply(net: &mut NetState, session: &mut Session, seq: u32, wire: WireMove) {
    if net.is_duplicate(seq) {
        return;
    }

    // The rules, not the sender, decide. A peer that is behind — or lying —
    // cannot push the game into a state the specification disallows.
    let Some(mv) = wire.resolve(&session.game.legal_moves()) else {
        warn!(?wire, seq, "dropping a sequenced move the rules reject");
        return;
    };

    session.game.play(&mv);
    net.last_applied_seq = Some(seq);
    session.selection = crate::Selection::None;
    after_turn(session);
}

/// No socket: apply straight away. Keeps the board playable rather than
/// silently swallowing moves.
fn apply_outbox_directly(session: &mut Session) {
    for mv in std::mem::take(&mut session.outbox) {
        if session.game.legal_moves().contains(&mv) {
            session.game.play(&mv);
            session.selection = crate::Selection::None;
            after_turn(session);
        }
    }
}

/// Audit the new position and pass over players with no legal move.
fn after_turn(session: &mut Session) {
    audit(session.game.position(), session.seating);

    while !session.game.is_over() && session.game.legal_moves().is_empty() {
        let stuck = session.game.turn();
        session.game.pass();
        session.message = format!("{} - player {} passed", session.message, stuck.index());
    }
}

//! In-game networking: submit local moves, apply sequenced ones.
//!
//! The one rule: **a move is applied only when it arrives sequenced**. Local
//! moves go into the outbox and come back through the same path as remote
//! ones, so every peer sees one ordering. Solo play takes the same path — the
//! lone peer is its own sequencer — so the networked code is always
//! exercised.

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_core::position::{MoveKind as GameMoveKind, Player};
use checkers_net::{CH_RELIABLE, NetMsg, NetState, WireMove, broadcast, decode, send_to};

use crate::{Session, audit};

/// Fold the socket's peer changes into [`NetState`]: connected peers are
/// added, disconnected ones dropped. Shared by the in-game pump and the
/// lobby's host election so the two can never disagree on who is present.
pub(crate) fn sync_peers(socket: &mut MatchboxSocket, net: &mut NetState) {
    for (peer, state) in socket.update_peers() {
        match state {
            PeerState::Connected => {
                if !net.peers.contains(&peer) {
                    info!(%peer, "peer connected");
                    net.peers.push(peer);
                }
            }
            PeerState::Disconnected => {
                if net.peers.contains(&peer) {
                    info!(%peer, "peer disconnected");
                    net.peers.retain(|p| *p != peer);
                }
            }
        }
    }
}

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

    sync_peers(&mut socket, &mut net);

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
            | NetMsg::Claim(_)
            | NetMsg::Hello { .. }
            | NetMsg::Roster(_)
            | NetMsg::Cursor { .. }
            | NetMsg::Variants { .. }
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
    if session.resolve(&wire).is_none() {
        warn!(?wire, "refusing to sequence a move the rules reject");
        return;
    }

    let seq = net.next_seq;
    net.next_seq += 1;
    broadcast(socket, peers, &NetMsg::Sequenced { seq, mv: wire });
    apply(net, session, seq, wire);
}

/// Apply a sequenced move, if it is new and legal.
pub(crate) fn apply(net: &mut NetState, session: &mut Session, seq: u32, wire: WireMove) {
    if net.is_duplicate(seq) {
        return;
    }

    // The rules, not the sender, decide. A peer that is behind — or lying —
    // cannot push the game into a state the specification disallows.
    let Some(mv) = session.resolve(&wire) else {
        warn!(?wire, seq, "dropping a sequenced move the rules reject");
        return;
    };

    let mover = session.game.turn();
    session.commit(&mv);
    net.last_applied_seq = Some(seq);
    session.selection = crate::Selection::None;
    log_move(net, mover, &wire, Some(seq));
    after_turn(session);
}

/// A move was played. `seq` is the host's sequence number in shared games;
/// solo play has none. The name comes from the roster, empty in hotseat play.
fn log_move(net: &NetState, mover: Player, wire: &WireMove, seq: Option<u32>) {
    let name: String = net
        .seats
        .iter()
        .find(|s| s.player == Some(mover.index() as u32))
        .map_or_else(String::new, |s| s.name.clone());
    info!(
        move_seq = seq,
        player = mover.index() + 1,
        name = %name,
        kind = if wire.jump { "jump" } else { "step" },
        from = %format!("({},{})", wire.origin.0, wire.origin.1),
        to = %format!("({},{})", wire.destination.0, wire.destination.1),
        "move applied",
    );
}

/// No socket: apply straight away. Keeps the board playable rather than
/// silently swallowing moves.
pub(crate) fn apply_outbox_directly(session: &mut Session) {
    for mv in std::mem::take(&mut session.outbox) {
        if !session.game.is_over() && session.game.legal_moves().contains(&mv) {
            let mover = session.game.turn();
            session.commit(&mv);
            session.selection = crate::Selection::None;
            info!(
                player = mover.index() + 1,
                kind = if mv.kind == GameMoveKind::Jump {
                    "jump"
                } else {
                    "step"
                },
                from = %format!("({},{})", mv.origin.q, mv.origin.r),
                to = %format!("({},{})", mv.destination.q, mv.destination.r),
                "move applied",
            );
            after_turn(session);
        }
    }
}

/// Audit the new position and pass over players with no legal move. The game's
/// end, however it came about, is logged here — the one point every live path
/// through a round reaches afterwards.
pub fn after_turn(session: &mut Session) {
    for stuck in settle(session) {
        info!(player = stuck.index() + 1, "no legal move - passes");
    }
    if let Some(outcome) = session.game.outcome() {
        log_outcome(outcome);
    }
}

/// [`after_turn`] without the story: audit, then pass over every player with
/// no legal move, and return who passed. Rebuilding a session from a record
/// settles each move this way, so replaying a round does not log its passes
/// and its ending a second time.
pub fn settle(session: &mut Session) -> Vec<Player> {
    audit(session.game.position(), &session.players);

    let mut passed = Vec::new();
    while !session.game.is_over() && session.game.legal_moves().is_empty() {
        let stuck = session.game.turn();
        session.game.pass();
        session.stats.passes += 1;
        session.message = format!("{} - player {} passed", session.message, stuck.index());
        passed.push(stuck);
    }
    passed
}

/// Log how the game ended. Called once from every path that ends the game —
/// [`after_turn`] after the move or pass that did it, and at the two endings
/// that skip it: a resignation and an engine abandonment.
pub fn log_outcome(outcome: checkers_core::rules::Outcome) {
    match outcome {
        checkers_core::rules::Outcome::Winner(p) => {
            info!(player = p.index() + 1, "game over: filled the target camp");
        }
        checkers_core::rules::Outcome::Resigned(p) => {
            info!(player = p.index() + 1, "game over: resigned");
        }
        checkers_core::rules::Outcome::Draw => info!("game over: draw - everyone is blocked"),
        checkers_core::rules::Outcome::Abandoned => {
            info!("game over: abandoned - the race stalled");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::Seating;

    /// A move that reaches a finished game — sequenced by the host, or still
    /// in the local outbox — is dropped. The winner's pieces still have moves
    /// after the winning one, and playing one would panic the game.
    #[test]
    fn a_move_after_the_end_is_dropped() {
        let mut session = Session::new(Seating::Two);
        let mv = session.game.legal_moves()[0].clone();
        session.game.abandon();

        let mut net = NetState::default();
        apply(&mut net, &mut session, 0, WireMove::from_move(&mv));
        session.outbox.push(mv);
        apply_outbox_directly(&mut session);

        assert!(session.history().is_empty(), "no move entered the round");
    }
}

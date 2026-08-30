//! Wire protocol and host-relay sequencing for networked play.
//!
//! Peer-to-peer over WebRTC via `bevy_matchbox`, with a signaling server used
//! only for introductions. There is no game server: one peer acts as **host**
//! and is the single sequencing authority.
//!
//! # Why a host relay
//!
//! Six players clicking simultaneously would otherwise apply moves in different
//! orders on different machines, and the rules are order-dependent — whether a
//! jump is available depends on where the blockers are. So:
//!
//! 1. A guest submits its move as [`NetMsg::Move`], to the host only.
//! 2. The host assigns the next sequence number and rebroadcasts
//!    [`NetMsg::Sequenced`] to everyone, *including itself*.
//! 3. Every peer — the originator included — applies a move only on
//!    `Sequenced`.
//!
//! Step 3 is the part that is easy to get wrong: if the host applied its own
//! moves directly it would run one move ahead of its guests, and the divergence
//! would only surface later as an illegal-move rejection. Routing the host's own
//! moves through the same sequencing arm makes one code path serialize
//! everything.
//!
//! # Trust
//!
//! [`WireMove`] carries only origin, destination, and kind — never a resulting
//! position. Receivers re-derive the move against their own
//! [`checkers_core::rules::legal_moves`], so a peer cannot induce a state the
//! rules disallow even if it sends nonsense. Host authority orders moves; the
//! rules, not the sender, decide whether one is legal.

use bevy::prelude::*;
use bevy_matchbox::prelude::*;
use checkers_core::geometry::Coord;
use checkers_core::position::{Move, MoveKind, Player};
use serde::{Deserialize, Serialize};

/// Signaling server used to introduce peers, set at compile time with
/// `MATCHBOX_SERVER`.
///
/// The default is **omdurman's** deployment, borrowed because it is already
/// running a compatible matchbox build. Two consequences worth knowing before
/// shipping this:
///
/// - Launching the app opens an outbound WebSocket to a host this project does
///   not control. Only peer-introduction traffic crosses it — game moves travel
///   peer-to-peer once WebRTC connects — but the connection itself is real.
/// - Room names share that server's namespace, so the default room
///   (`"checkers"`) could collide with anyone else using it.
///
/// Point `MATCHBOX_SERVER` at your own `matchbox_server` for anything beyond
/// local testing.
pub const SIGNALING_SERVER: &str = match option_env!("MATCHBOX_SERVER") {
    Some(s) => s,
    None => "wss://omdurman-matchbox.fly.dev",
};

/// Reliable, ordered channel. Everything here is game-mutating, so there is no
/// unreliable channel — unlike omdurman, this game has no cursors to stream.
pub const CH_RELIABLE: usize = 0;

/// A move as it travels on the wire.
///
/// Deliberately *not* [`Move`]: that type carries a presentational `route` and
/// is not serialisable. Sending only the identity triple `(kind, origin,
/// destination)` matches chapter 10's rule that a move *is* its endpoints, so
/// two routes to the same hole cannot arrive as different moves.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct WireMove {
    pub origin: (i32, i32),
    pub destination: (i32, i32),
    pub jump: bool,
}

impl WireMove {
    pub fn from_move(mv: &Move) -> Self {
        Self {
            origin: (mv.origin.q, mv.origin.r),
            destination: (mv.destination.q, mv.destination.r),
            jump: mv.kind == MoveKind::Jump,
        }
    }

    pub fn origin_coord(&self) -> Coord {
        Coord::new(self.origin.0, self.origin.1)
    }

    pub fn destination_coord(&self) -> Coord {
        Coord::new(self.destination.0, self.destination.1)
    }

    /// Find this move among `legal`, or `None` if no legal move matches.
    ///
    /// The only way a wire move becomes a [`Move`]: an unmatched triple is
    /// dropped rather than constructed, so a malicious or out-of-sync peer
    /// cannot push the game off the rules.
    pub fn resolve(&self, legal: &[Move]) -> Option<Move> {
        let kind = if self.jump {
            MoveKind::Jump
        } else {
            MoveKind::Step
        };
        legal
            .iter()
            .find(|m| {
                m.kind == kind
                    && m.origin == self.origin_coord()
                    && m.destination == self.destination_coord()
            })
            .cloned()
    }
}

/// Lobby-visible player slot.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    /// Stable within a session; `PeerId`'s string form so it survives postcard.
    pub peer: String,
    pub name: String,
    /// Which of the six players this peer commands, once the host assigns it.
    pub player: Option<u32>,
    pub ready: bool,
}

/// Top-level wire envelope.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum NetMsg {
    /// Guest -> host: an unsequenced submission. Never applied directly.
    Move(WireMove),
    /// Host -> all: the canonical ordered form. The only one that is applied.
    Sequenced { seq: u32, mv: WireMove },
    /// Introduce yourself on join.
    Hello { name: String },
    /// Host -> all: the full lobby roster, whenever it changes.
    Roster(Vec<Seat>),
    /// Guest -> host: toggle my ready flag.
    Ready(bool),
    /// Host -> all: assignments are final, start playing.
    Start { seats: Vec<Seat> },
}

pub fn encode(msg: &NetMsg) -> Option<Box<[u8]>> {
    match postcard::to_allocvec(msg) {
        // A WebRTC data channel can silently drop a zero-byte payload:
        // `try_send` returns Ok, but the receiver never fires. Any real NetMsg
        // encodes to at least a variant tag, so this only happens on encode
        // failure — skip the send rather than put an invisible packet on the wire.
        Ok(v) if !v.is_empty() => Some(v.into_boxed_slice()),
        Ok(_) => {
            error!("postcard produced an empty NetMsg encoding; dropping");
            None
        }
        Err(error) => {
            error!(%error, "postcard encode failed");
            None
        }
    }
}

pub fn decode(raw: &[u8]) -> Option<NetMsg> {
    postcard::from_bytes(raw)
        .inspect_err(|error| warn!(%error, "NetMsg decode failed"))
        .ok()
}

/// Everything the front-end needs to know about the connection.
#[derive(Resource, Default)]
pub struct NetState {
    pub peers: Vec<PeerId>,
    pub my_id: Option<PeerId>,
    pub is_host: bool,
    pub seats: Vec<Seat>,
    /// Host-only: next sequence number to assign.
    pub next_seq: u32,
    /// Highest applied sequence number, for dropping duplicate deliveries.
    pub last_applied_seq: Option<u32>,
    pub name: String,
    /// Human-readable explanation of the last refused action, shown in the
    /// lobby. Empty when there is nothing to explain.
    pub status: String,
    /// Peers we have already sent our [`NetMsg::Hello`] to. Without this a
    /// per-frame greet loop would flood the channel; disconnected peers may
    /// stay listed here — a reconnect arrives with a fresh id anyway.
    pub greeted: Vec<PeerId>,
}

impl NetState {
    /// Am I the sequencing authority? True for the host, and for a solo peer so
    /// that a single player can start before anyone else joins.
    pub fn sequences(&self) -> bool {
        self.is_host || self.peers.is_empty()
    }

    /// Which [`Player`] this peer commands, if the host has assigned one.
    pub fn my_player(&self) -> Option<Player> {
        let me = self.my_id?.to_string();
        self.seats
            .iter()
            .find(|s| s.peer == me)
            .and_then(|s| s.player)
            .and_then(|i| Player::ALL.get(i as usize).copied())
    }

    /// Has this sequence number already been applied? The reliable channel is
    /// ordered and `seq` is monotonic, so anything at or below the high-water
    /// mark is a duplicate.
    pub fn is_duplicate(&self, seq: u32) -> bool {
        self.last_applied_seq.is_some_and(|last| seq <= last)
    }
}

/// The room to join. Peers sharing a room id find each other.
#[derive(Resource, Clone)]
pub struct RoomId(pub String);

impl Default for RoomId {
    fn default() -> Self {
        // Namespaced, because the default signaling server is shared with other
        // projects. The bare name `checkers` collided with live sessions there:
        // two unrelated peers joined, this build lost the host election, and it
        // sat in the lobby forever waiting for a `Start` that those peers — a
        // different game entirely — were never going to send.
        Self("chinese-checkers-rs-v1".into())
    }
}

pub fn open_socket(mut commands: Commands, room: Res<RoomId>) {
    let url = format!("{SIGNALING_SERVER}/{}", room.0);
    info!(%url, "opening matchbox socket");
    commands.insert_resource(MatchboxSocket::from(
        WebRtcSocketBuilder::new(url)
            .reconnect_attempts(None)
            .add_reliable_channel(),
    ));
}

/// Send to one peer on the reliable channel.
pub fn send_to(socket: &mut MatchboxSocket, peer: PeerId, msg: &NetMsg) {
    let Some(bytes) = encode(msg) else {
        return;
    };
    if let Err(error) = socket.channel_mut(CH_RELIABLE).try_send(bytes, peer) {
        warn!(%error, "send failed");
    }
}

/// Send to every connected peer.
pub fn broadcast(socket: &mut MatchboxSocket, peers: &[PeerId], msg: &NetMsg) {
    let Some(bytes) = encode(msg) else {
        return;
    };
    for &peer in peers {
        if let Err(error) = socket
            .channel_mut(CH_RELIABLE)
            .try_send(bytes.clone(), peer)
        {
            warn!(%error, %peer, "send failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use checkers_core::rules::legal_moves;

    #[test]
    fn a_wire_move_round_trips_through_the_legal_move_list() {
        let pos = checkers_core::position::Position::initial();
        let legal = legal_moves(&pos, Player::ALL[0]);
        let mv = legal.first().expect("the initial position has moves");

        let wire = WireMove::from_move(mv);
        let resolved = wire.resolve(&legal).expect("its own move must resolve");
        assert_eq!(resolved.origin, mv.origin);
        assert_eq!(resolved.destination, mv.destination);
        assert_eq!(resolved.kind, mv.kind);
    }

    #[test]
    fn an_illegal_wire_move_does_not_resolve() {
        let pos = checkers_core::position::Position::initial();
        let legal = legal_moves(&pos, Player::ALL[0]);

        // Structurally well-formed, but not a legal move in this position.
        let bogus = WireMove {
            origin: (0, 0),
            destination: (0, 1),
            jump: false,
        };
        assert!(
            bogus.resolve(&legal).is_none(),
            "an unmatched triple must be dropped, not constructed"
        );
    }

    #[test]
    fn a_step_and_a_jump_to_the_same_hole_are_distinct_on_the_wire() {
        let a = WireMove {
            origin: (0, 0),
            destination: (2, 0),
            jump: true,
        };
        let b = WireMove { jump: false, ..a };
        assert_ne!(a, b);
    }

    #[test]
    fn encode_decode_round_trips() {
        let msg = NetMsg::Sequenced {
            seq: 7,
            mv: WireMove {
                origin: (1, -2),
                destination: (3, -4),
                jump: true,
            },
        };
        let bytes = encode(&msg).expect("encodes");
        let Some(NetMsg::Sequenced { seq, mv }) = decode(&bytes) else {
            panic!("decoded to the wrong variant");
        };
        assert_eq!(seq, 7);
        assert_eq!(mv.origin, (1, -2));
        assert!(mv.jump);
    }

    #[test]
    fn decoding_garbage_yields_none() {
        assert!(decode(&[0xff, 0xff, 0xff, 0xff]).is_none());
    }

    #[test]
    fn duplicate_sequence_numbers_are_recognised() {
        let mut net = NetState::default();
        assert!(!net.is_duplicate(0), "nothing applied yet");
        net.last_applied_seq = Some(3);
        assert!(net.is_duplicate(3), "the same seq is a duplicate");
        assert!(net.is_duplicate(2), "an older seq is a duplicate");
        assert!(!net.is_duplicate(4), "the next seq is new");
    }

    /// Only the no-peer arms; the `peers` branch needs a real `PeerId`, which
    /// requires a `uuid` dev-dependency for no gain — the guest-with-peers path
    /// is covered end-to-end by `sequencing_is_the_only_path_to_the_game` in
    /// `checkers-bevy/tests/net.rs`.
    #[test]
    fn a_solo_peer_sequences_whether_or_not_it_is_host() {
        let mut net = NetState::default();
        assert!(net.sequences(), "a lone guest must be able to start");
        net.is_host = true;
        assert!(net.sequences(), "and so must a lone host");
    }

    #[test]
    fn my_player_is_none_until_the_host_assigns_a_seat() {
        let net = NetState::default();
        assert!(net.my_player().is_none(), "no id and no seats yet");
    }
}

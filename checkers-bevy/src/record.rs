//! The `.cchkrs` game record: a round saved as text, resumed by replay.
//!
//! A record names the seating and lists the moves in play order — nothing
//! else. The position is not stored, because it is derived: the composed
//! game's initial position is a function of the seating, and every recorded
//! move is re-resolved against the rules on replay (`WireMove::resolve`),
//! so a record cannot smuggle the game into a position the specification
//! disallows. Chapter 10 is why the moves are wire moves: the route is not
//! part of a move's identity, so the record does not carry routes, and a
//! resumed game's hop statistics follow the rebuilt routes rather than the
//! originally flown ones. Move counts, jump counts, and passes reproduce
//! exactly.
//!
//! The format is line-based text, versioned in its first line, so a record
//! written by an older build is rejected with a readable fault rather than
//! parsed into something wrong.

use crate::setup::Seating;
use checkers_core::position::Player;
use checkers_net::WireMove;

/// The first line every record starts with. Bump on any incompatible change.
const HEADER: &str = "cchkrs 1";

/// A saved round: which camps sat down, which seats the computer plays, and
/// the moves in play order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRecord {
    pub seating: Seating,
    /// Seats the engine plays in the resumed game. Empty means every seated
    /// camp is human.
    pub ai_players: Vec<Player>,
    pub moves: Vec<WireMove>,
}

/// Why a record could not be read. Every variant is meant to reach the
/// player as text, so they say what was wrong rather than where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordFault {
    /// The first line is not the expected header.
    Header(String),
    /// The `seating` line names camps that are no seating this build knows.
    Seating(String),
    /// A move line does not parse.
    Move { line: usize, text: String },
    /// The declared move count disagrees with the move lines.
    Count { declared: usize, found: usize },
    /// A recorded move was not legal in the position it occurs in — the
    /// replay refused it.
    Replay { ply: usize, why: String },
}

impl core::fmt::Display for RecordFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecordFault::Header(line) => {
                write!(f, "not a saved game: expected \"{HEADER}\", found {line:?}")
            }
            RecordFault::Seating(line) => write!(f, "unknown seating: {line:?}"),
            RecordFault::Move { line, text } => {
                write!(f, "move {line} does not parse: {text:?}")
            }
            RecordFault::Count { declared, found } => write!(
                f,
                "the header says {declared} moves, but the record holds {found}"
            ),
            RecordFault::Replay { ply, why } => {
                write!(f, "move {} is not legal: {}", ply + 1, why)
            }
        }
    }
}

impl core::error::Error for RecordFault {}

impl GameRecord {
    /// The record as `.cchkrs` text.
    pub fn to_text(&self) -> String {
        let mut out = String::from(HEADER);
        out.push_str("\nseating");
        for i in self.seating.indices() {
            out.push_str(&format!(" {i}"));
        }
        out.push_str("\nai");
        if self.ai_players.is_empty() {
            out.push_str(" -");
        } else {
            for p in &self.ai_players {
                out.push_str(&format!(" {}", p.index()));
            }
        }
        out.push_str(&format!("\nmoves {}", self.moves.len()));
        for mv in &self.moves {
            let kind = if mv.jump { 'j' } else { 's' };
            out.push_str(&format!(
                "\n{kind} {},{} {},{}",
                mv.origin.0, mv.origin.1, mv.destination.0, mv.destination.1
            ));
        }
        out
    }

    /// Parse `.cchkrs` text. Syntax only — the moves are validated against
    /// the rules when the session replays them, which is where a forged or
    /// corrupted record is actually caught.
    pub fn from_text(text: &str) -> Result<Self, RecordFault> {
        let mut lines = text.lines().enumerate();
        let (_, first) = lines
            .next()
            .ok_or_else(|| RecordFault::Header(String::new()))?;
        if first.trim() != HEADER {
            return Err(RecordFault::Header(first.to_string()));
        }

        let mut seating = None;
        let mut ai_players = Vec::new();
        let mut declared = None;
        let mut moves = Vec::new();

        for (n, line) in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            match word {
                "seating" => {
                    let indices: Vec<u32> = rest
                        .split_whitespace()
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    seating =
                        Some(Seating::from_indices(&indices).ok_or_else(|| {
                            RecordFault::Seating(format!("line {}: {line}", n + 1))
                        })?);
                }
                "ai" => {
                    ai_players = rest
                        .split_whitespace()
                        .filter(|t| *t != "-")
                        .filter_map(|t| t.parse::<u8>().ok())
                        .filter_map(Player::new)
                        .collect();
                }
                "moves" => {
                    declared = Some(rest.trim().parse().map_err(|_| RecordFault::Move {
                        line: n + 1,
                        text: line.to_string(),
                    })?);
                }
                "s" | "j" => {
                    moves.push(
                        parse_move(word == "j", rest).ok_or_else(|| RecordFault::Move {
                            line: n + 1,
                            text: line.to_string(),
                        })?,
                    );
                }
                _ => {
                    return Err(RecordFault::Move {
                        line: n + 1,
                        text: line.to_string(),
                    });
                }
            }
        }

        let seating =
            seating.ok_or_else(|| RecordFault::Seating("the record names no seating".into()))?;
        let declared = declared.unwrap_or(moves.len());
        if declared != moves.len() {
            return Err(RecordFault::Count {
                declared,
                found: moves.len(),
            });
        }
        Ok(Self {
            seating,
            ai_players,
            moves,
        })
    }
}

/// Parse one `q,r q,r` pair of coordinates. Out-of-range coordinates fail
/// here; coordinates off the board are caught by the replay's legality
/// check, which is the authority.
fn parse_move(jump: bool, rest: &str) -> Option<WireMove> {
    let mut parts = rest.split_whitespace();
    let origin = parts.next()?.split_once(',')?;
    let destination = parts.next()?.split_once(',')?;
    Some(WireMove {
        origin: (origin.0.parse().ok()?, origin.1.parse().ok()?),
        destination: (destination.0.parse().ok()?, destination.1.parse().ok()?),
        jump,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;
    use checkers_core::rules::legal_moves;
    use checkers_core::turn::step_destinations;

    /// A record of a real round: play one step, save, parse, resume — and
    /// the resumed session must be indistinguishable in every way that
    /// matters: position, turn, history.
    #[test]
    fn a_round_trips_through_text_unchanged() {
        let mut session = crate::Session::new(Seating::Two);
        let player = session.game.turn();
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
        session.confirm();
        crate::net::apply_outbox_directly(&mut session);

        let record = session.to_record();
        let text = record.to_text();
        let parsed = GameRecord::from_text(&text).expect("our own record must parse");
        assert_eq!(parsed, record);

        let resumed = crate::Session::resumed(&parsed).expect("our own record must resume");
        assert_eq!(resumed.game.position(), session.game.position());
        assert_eq!(resumed.game.turn(), session.game.turn());
        assert_eq!(resumed.game.outcome(), session.game.outcome());
        assert_eq!(resumed.stats.total_moves(), session.stats.total_moves());
    }

    /// A record from another table — different move order, forged positions,
    /// garbage — is refused with a fault that names the problem.
    #[test]
    fn a_corrupted_record_is_refused() {
        let no_header = GameRecord::from_text("hello\n");
        assert!(matches!(no_header, Err(RecordFault::Header(_))));

        let no_seating = GameRecord::from_text("cchkrs 1\nmoves 0\n");
        assert!(matches!(no_seating, Err(RecordFault::Seating(_))));

        let bad_move = GameRecord::from_text("cchkrs 1\nseating 0 3\nmoves 1\nx 0,0 1,1\n");
        assert!(matches!(bad_move, Err(RecordFault::Move { line: 4, .. })));

        let bad_count = GameRecord::from_text("cchkrs 1\nseating 0 3\nmoves 2\n");
        assert!(matches!(
            bad_count,
            Err(RecordFault::Count {
                declared: 2,
                found: 0
            })
        ));

        let bad_seating = GameRecord::from_text("cchkrs 1\nseating 0 1\nmoves 0\n");
        assert!(matches!(bad_seating, Err(RecordFault::Seating(_))));
    }

    /// A record whose moves are not legal where they occur is rejected by
    /// the replay, not by the parser — the rules, not the text, are the
    /// authority.
    #[test]
    fn an_illegal_recorded_move_is_rejected_on_resume() {
        let text = "cchkrs 1\nseating 0 3\nmoves 1\nj 0,5 0,1\n";
        let record = GameRecord::from_text(text).expect("the line itself parses");
        let resumed = crate::Session::resumed(&record);
        let Err(fault) = resumed else {
            panic!("teleporting a piece is not a legal move");
        };
        let text = fault.to_string();
        assert!(
            text.contains("not legal"),
            "the fault should say the move was rejected: {text}"
        );
    }

    /// The replay runs the law audit after every move, so a record that
    /// would produce a position violating the invariants cannot come back
    /// as a session. (The legality check above already refuses most of
    /// those; this is the belt to its braces.)
    #[test]
    fn a_resumed_position_passes_the_audit() {
        let session = crate::Session::new(Seating::Six);
        let resumed = crate::Session::resumed(&session.to_record())
            .expect("an empty record of a real deal must resume");
        crate::audit(resumed.game.position(), resumed.seating);
        assert!(!legal_moves(resumed.game.position(), resumed.game.turn()).is_empty());
        assert!(matches!(resumed.selection, Selection::None));
    }
}

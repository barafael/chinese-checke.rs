//! Validating a *live* position against the specification.
//!
//! [`crate::law::LAWS`] answers a different question: each law generates its own
//! subjects and checks itself, which is what a test suite wants but is far too
//! slow to run per move — every law rebuilds sample games. It also never sees the
//! caller's position.
//!
//! [`audit_position`] instead applies the position invariants to one given
//! position. It is O(holes) and safe to call on every state change, which is what
//! a front-end needs in order to fail loudly on a corrupted board.
//!
//! Only the checks that can actually fire live here. "At most one piece per
//! hole" and "no piece off the board" are structural: a [`Position`] is a map
//! from board holes to occupants, and off-board writes panic in
//! [`Position::set`]. Occupied and empty counts are arithmetic corollaries of
//! the six per-player counts. Adding checks for those would be decoration, not
//! validation.

use crate::position::{HOLES, PIECES_PER_PLAYER, Player, Position};

/// A position invariant that was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionFault {
    /// A player owns the wrong number of pieces.
    PieceCount { player: u8, found: usize },
    /// The position is not backed by the 121-hole board.
    HoleTable { found: usize },
}

impl core::fmt::Display for PositionFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PositionFault::PieceCount { player, found } => write!(
                f,
                "player {player} owns {found} pieces, expected {PIECES_PER_PLAYER} \
                 (law CC-POS-PIECES)"
            ),
            PositionFault::HoleTable { found } => write!(
                f,
                "the hole table has {found} entries, expected {HOLES} \
                 (law CC-GEO-CARDINALITY)"
            ),
        }
    }
}

impl core::error::Error for PositionFault {}

/// Check one position against the invariants of chapters 6 and 14.
///
/// Linear in the number of holes, so callers may run it on every state change.
/// Use this in a front-end; use [`crate::law::verify_all`] in tests.
pub fn audit_position(pos: &Position) -> Result<(), PositionFault> {
    for player in Player::ALL {
        let n = pos.count_of(player);
        if n != PIECES_PER_PLAYER {
            return Err(PositionFault::PieceCount {
                player: player.index(),
                found: n,
            });
        }
    }

    if pos.holes().len() != HOLES {
        return Err(PositionFault::HoleTable {
            found: pos.holes().len(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position::PLAYERS;
    use crate::rules::{apply, legal_moves};

    #[test]
    fn the_initial_position_passes() {
        assert_eq!(audit_position(&Position::initial()), Ok(()));
    }

    #[test]
    fn positions_reached_by_play_pass() {
        let mut pos = Position::initial();
        for ply in 0..40 {
            let player = Player::wrapping((ply % PLAYERS) as u8);
            let moves = legal_moves(&pos, player);
            if moves.is_empty() {
                continue;
            }
            pos = apply(&pos, &moves[0]);
            assert_eq!(audit_position(&pos), Ok(()), "failed at ply {ply}");
        }
    }

    #[test]
    fn a_removed_piece_is_caught() {
        let mut pos = Position::initial();
        let victim = Player::ALL[2].start_camp()[0];
        pos.set(victim, None);

        assert_eq!(
            audit_position(&pos),
            Err(PositionFault::PieceCount {
                player: 2,
                found: 9
            })
        );
    }

    #[test]
    fn a_duplicated_piece_is_caught() {
        let mut pos = Position::initial();
        // Overwrite one player's piece with another's: counts skew both ways.
        let target = Player::ALL[1].start_camp()[0];
        pos.set(target, Some(Player::ALL[0]));

        let fault = audit_position(&pos).expect_err("should be caught");
        assert!(matches!(fault, PositionFault::PieceCount { .. }), "{fault}");
    }

    #[test]
    fn an_empty_board_is_caught() {
        assert!(audit_position(&Position::empty()).is_err());
    }

    #[test]
    fn faults_describe_themselves_with_the_law_id() {
        let f = PositionFault::PieceCount {
            player: 3,
            found: 11,
        };
        let text = f.to_string();
        assert!(text.contains("player 3"), "{text}");
        assert!(text.contains("CC-POS-PIECES"), "{text}");
    }

    /// The audit must be fast enough to run per move; this would take minutes
    /// if it re-ran the whole law registry.
    #[test]
    fn auditing_is_cheap() {
        let pos = Position::initial();
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            audit_position(&pos).unwrap();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 2000,
            "1000 audits took {elapsed:?}, too slow for per-move use"
        );
    }
}

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
    /// A player not in the game holds pieces — they would sit forever on
    /// holes another player needs to win through.
    GhostPiece { player: u8, found: usize },
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
            PositionFault::GhostPiece { player, found } => write!(
                f,
                "player {player} is not in the game yet owns {found} pieces \
                 (chapter 15: unseated players sit out entirely)"
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

/// Check one position against the invariants of chapters 6, 14, and 15.
///
/// Every player in `players` must own exactly [`PIECES_PER_PLAYER`] pieces and
/// every other player none. Passing [`Player::ALL`] checks the specification's
/// six-player invariant exactly; a composed smaller game checks its own
/// players and requires the vacant seats to be empty — a ghost camp would
/// block the target camp of whoever must cross it.
///
/// Linear in the number of holes, so callers may run it on every state change.
/// Use this in a front-end; use [`crate::law::verify_all`] in tests.
pub fn audit_position(pos: &Position, players: &[Player]) -> Result<(), PositionFault> {
    for player in Player::ALL {
        let n = pos.count_of(player);
        if players.contains(&player) {
            if n != PIECES_PER_PLAYER {
                return Err(PositionFault::PieceCount {
                    player: player.index(),
                    found: n,
                });
            }
        } else if n != 0 {
            return Err(PositionFault::GhostPiece {
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
        assert_eq!(audit_position(&Position::initial(), &Player::ALL), Ok(()));
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
            assert_eq!(
                audit_position(&pos, &Player::ALL),
                Ok(()),
                "failed at ply {ply}"
            );
        }
    }

    #[test]
    fn a_removed_piece_is_caught() {
        let mut pos = Position::initial();
        let victim = Player::ALL[2].start_camp()[0];
        pos.set(victim, None);

        assert_eq!(
            audit_position(&pos, &Player::ALL),
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

        let fault = audit_position(&pos, &Player::ALL).expect_err("should be caught");
        assert!(matches!(fault, PositionFault::PieceCount { .. }), "{fault}");
    }

    #[test]
    fn an_empty_board_is_caught() {
        assert!(audit_position(&Position::empty(), &Player::ALL).is_err());
    }

    /// A composed two-player position satisfies chapter 14 for its own players
    /// — and must, or every networked two-player game would panic the
    /// front-end's per-move audit.
    #[test]
    fn a_two_player_position_passes_for_its_players() {
        let two = [Player::ALL[0], Player::ALL[1]];
        let pos = crate::rules::Game::for_players(&two);
        assert_eq!(audit_position(pos.position(), &two), Ok(()));
    }

    /// A ghost camp is caught: pieces of a player not in the game would sit
    /// forever on holes a seated player must cross to win.
    #[test]
    fn a_ghost_player_is_caught() {
        let two = [Player::ALL[0], Player::ALL[1]];
        let game = crate::rules::Game::for_players(&two);
        assert_eq!(
            audit_position(game.position(), &two),
            Ok(()),
            "the composed position is clean"
        );
        assert!(matches!(
            audit_position(&Position::initial(), &two),
            Err(PositionFault::GhostPiece { player: 2, .. })
        ));
    }

    /// Symmetrically, a seated player missing pieces is still caught when
    /// other seats are vacant — the count fault outranks the vacancy.
    #[test]
    fn a_short_handed_active_player_is_caught() {
        let game = crate::rules::Game::for_players(&[Player::ALL[0], Player::ALL[1]]);
        let mut pos = game.position().clone();
        let victim = Player::ALL[0].start_camp()[0];
        pos.set(victim, None);
        assert!(matches!(
            audit_position(&pos, &[Player::ALL[0], Player::ALL[1]]),
            Err(PositionFault::PieceCount { player: 0, found: 9 })
        ));
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
            audit_position(&pos, &Player::ALL).unwrap();
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 2000,
            "1000 audits took {elapsed:?}, too slow for per-move use"
        );
    }
}

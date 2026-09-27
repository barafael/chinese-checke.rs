//! The append-only move log for games a human wants to read after the fact.
//!
//! Native builds append to `moves.log` next to the working directory; web
//! builds have no filesystem, so lines go to the console log instead.
//!
//! [`describe`] is how a move reads in both logs: this file and the game-story
//! lines the network code writes through `tracing`.

use checkers_core::position::{Move, MoveKind};

#[cfg(not(target_family = "wasm"))]
pub fn log(line: &str) {
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("moves.log")
    else {
        return;
    };
    let _ = writeln!(file, "{line}");
}

#[cfg(target_family = "wasm")]
pub fn log(line: &str) {
    bevy::log::info!("{line}");
}

/// Describe a move the way the log reads it.
pub fn describe(mv: &Move) -> String {
    let kind = match mv.kind {
        MoveKind::Step => "step",
        MoveKind::Jump => "jump",
    };
    let mut out = format!(
        "{kind} ({},{}) -> ({},{})",
        mv.origin.q, mv.origin.r, mv.destination.q, mv.destination.r
    );
    if let Some(route) = &mv.route
        && route.len() > 2
    {
        let via: Vec<String> = route[1..route.len() - 1]
            .iter()
            .map(|c| format!("({},{})", c.q, c.r))
            .collect();
        out.push_str(&format!(" via {}", via.join(", ")));
    }
    out
}

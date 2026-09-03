//! The append-only move log for games a human wants to read after the fact.
//!
//! Native builds append to `moves.log` next to the working directory; web
//! builds have no filesystem, so lines go to the console log instead.

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

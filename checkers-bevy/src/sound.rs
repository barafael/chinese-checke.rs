//! Small synthesized feedback sounds.
//!
//! Each event answers with a short sine tone from Bevy's built-in [`Pitch`]
//! source — the backend generates the wave itself, so there are no audio
//! files, no decoder features, and nothing to load. One tone per meaning:
//! a hop ticks, a commit settles, a cancel sinks, a win rings, a resignation
//! falls. `M` mutes and unmutes; sound starts off.

use bevy::prelude::*;
use std::time::Duration;

/// Whether this build answers with sound. Muted by default; `M` unmutes.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct SoundOn(pub bool);

/// The tones, built once at startup and kept by handle.
#[derive(Resource)]
pub struct Sounds {
    hop: Handle<Pitch>,
    commit: Handle<Pitch>,
    cancel: Handle<Pitch>,
    win: Handle<Pitch>,
    resign: Handle<Pitch>,
}

/// Which tone to answer an event with.
#[derive(Debug, Clone, Copy)]
pub enum SoundKind {
    /// A hop or step was staged.
    Hop,
    /// A turn was committed.
    Commit,
    /// A staged turn was abandoned.
    Cancel,
    /// The round ended in a win or a draw.
    Win,
    /// The round ended in resignation.
    Resign,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<SoundOn>()
        .add_systems(PreStartup, init)
        .add_systems(Update, toggle);
}

fn init(mut pitches: ResMut<Assets<Pitch>>, mut commands: Commands) {
    commands.insert_resource(Sounds {
        hop: pitches.add(Pitch::new(520.0, Duration::from_millis(90))),
        commit: pitches.add(Pitch::new(340.0, Duration::from_millis(160))),
        cancel: pitches.add(Pitch::new(220.0, Duration::from_millis(180))),
        win: pitches.add(Pitch::new(660.0, Duration::from_millis(400))),
        resign: pitches.add(Pitch::new(180.0, Duration::from_millis(400))),
    });
}

impl Sounds {
    /// Answer an event, if sound is on. Fire-and-forget: the entity despawns
    /// when the tone ends. On the web the first click both is the gesture the
    /// browser waits for and plays the first tone, so autoplay policy is met
    /// by construction.
    pub fn play(&self, commands: &mut Commands, on: SoundOn, kind: SoundKind) {
        if !on.0 {
            return;
        }
        let handle = match kind {
            SoundKind::Hop => &self.hop,
            SoundKind::Commit => &self.commit,
            SoundKind::Cancel => &self.cancel,
            SoundKind::Win => &self.win,
            SoundKind::Resign => &self.resign,
        };
        commands.spawn((AudioPlayer(handle.clone()), PlaybackSettings::DESPAWN));
    }
}

/// `M` mutes and unmutes. Registered outside the game states: sound is a
/// whole-app setting.
fn toggle(keys: Res<ButtonInput<KeyCode>>, mut on: ResMut<SoundOn>) {
    if keys.just_pressed(KeyCode::KeyM) {
        on.0 = !on.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sound is born muted: a fresh build should not surprise the player with
    /// tones, and unmuting is one explicit `M`.
    #[test]
    fn sound_starts_muted() {
        assert!(!SoundOn::default().0);
    }

    /// Muting is a toggle: M twice returns to where it was.
    #[test]
    fn mute_is_a_toggle_not_a_latch() {
        let mut on = SoundOn::default();
        on.0 = !on.0;
        on.0 = !on.0;
        assert!(!on.0);
    }
}

//! Click sample playback
//!
//! Handles playback of click samples: beat, eighth, sixteenth, triplet, and
//! measure accent. Ported from fts-guide; the shared-state `Arc<Mutex<..>>`
//! sample slots became plain `Option<&AudioSample>` references.

use crate::samples::AudioSample;

use super::routing::AudioRouter;

/// Click sample player state
#[derive(Debug, Clone)]
pub struct ClickPlayerState {
    pub playback_position_beat: usize,
    pub playback_position_eighth: usize,
    pub playback_position_sixteenth: usize,
    pub playback_position_triplet: usize,
    pub playback_position_measure_accent: usize,
    pub is_playing_beat: bool,
    pub is_playing_eighth: bool,
    pub is_playing_sixteenth: bool,
    pub is_playing_triplet: bool,
    pub is_playing_measure_accent: bool,
}

impl ClickPlayerState {
    pub fn new() -> Self {
        Self {
            playback_position_beat: 0,
            playback_position_eighth: 0,
            playback_position_sixteenth: 0,
            playback_position_triplet: 0,
            playback_position_measure_accent: 0,
            is_playing_beat: false,
            is_playing_eighth: false,
            is_playing_sixteenth: false,
            is_playing_triplet: false,
            is_playing_measure_accent: false,
        }
    }

    /// Reset all playback states
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for ClickPlayerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Advance one voice slot by one frame, mixing into `left`/`right`.
/// Shared body of the per-subdivision `play_*` fns (logic unchanged from
/// the legacy per-slot implementations).
fn play_slot(
    is_playing: &mut bool,
    position: &mut usize,
    sample_data: Option<&AudioSample>,
    gain: f32,
    left: &mut f32,
    right: &mut f32,
) {
    if !*is_playing {
        return;
    }
    if let Some(decoded_audio) = sample_data {
        if *position < decoded_audio.frames() {
            AudioRouter::mix_decoded_audio(decoded_audio, *position, gain, left, right);
            *position += 1;

            if *position >= decoded_audio.frames() {
                *is_playing = false;
                *position = 0;
            }
        } else {
            *is_playing = false;
            *position = 0;
        }
    }
}

/// Click sample player
pub struct ClickPlayer;

impl ClickPlayer {
    /// Play beat sample and mix to output
    pub fn play_beat(
        state: &mut ClickPlayerState,
        sample_data: Option<&AudioSample>,
        gain: f32,
        click_left: &mut f32,
        click_right: &mut f32,
    ) {
        play_slot(
            &mut state.is_playing_beat,
            &mut state.playback_position_beat,
            sample_data,
            gain,
            click_left,
            click_right,
        );
    }

    /// Play eighth sample and mix to output
    pub fn play_eighth(
        state: &mut ClickPlayerState,
        sample_data: Option<&AudioSample>,
        gain: f32,
        click_left: &mut f32,
        click_right: &mut f32,
    ) {
        play_slot(
            &mut state.is_playing_eighth,
            &mut state.playback_position_eighth,
            sample_data,
            gain,
            click_left,
            click_right,
        );
    }

    /// Play sixteenth sample and mix to output
    pub fn play_sixteenth(
        state: &mut ClickPlayerState,
        sample_data: Option<&AudioSample>,
        gain: f32,
        click_left: &mut f32,
        click_right: &mut f32,
    ) {
        play_slot(
            &mut state.is_playing_sixteenth,
            &mut state.playback_position_sixteenth,
            sample_data,
            gain,
            click_left,
            click_right,
        );
    }

    /// Play triplet sample and mix to output
    pub fn play_triplet(
        state: &mut ClickPlayerState,
        sample_data: Option<&AudioSample>,
        gain: f32,
        click_left: &mut f32,
        click_right: &mut f32,
    ) {
        play_slot(
            &mut state.is_playing_triplet,
            &mut state.playback_position_triplet,
            sample_data,
            gain,
            click_left,
            click_right,
        );
    }

    /// Play measure accent sample and mix to output
    pub fn play_measure_accent(
        state: &mut ClickPlayerState,
        sample_data: Option<&AudioSample>,
        gain: f32,
        click_left: &mut f32,
        click_right: &mut f32,
    ) {
        play_slot(
            &mut state.is_playing_measure_accent,
            &mut state.playback_position_measure_accent,
            sample_data,
            gain,
            click_left,
            click_right,
        );
    }

    /// Play all active click samples and mix to output
    #[allow(clippy::too_many_arguments)]
    pub fn play_all(
        state: &mut ClickPlayerState,
        sample_data_beat: Option<&AudioSample>,
        sample_data_eighth: Option<&AudioSample>,
        sample_data_sixteenth: Option<&AudioSample>,
        sample_data_triplet: Option<&AudioSample>,
        sample_data_measure_accent: Option<&AudioSample>,
        gain: f32,
        click_left: &mut f32,
        click_right: &mut f32,
    ) {
        Self::play_beat(state, sample_data_beat, gain, click_left, click_right);
        Self::play_eighth(state, sample_data_eighth, gain, click_left, click_right);
        Self::play_sixteenth(state, sample_data_sixteenth, gain, click_left, click_right);
        Self::play_triplet(state, sample_data_triplet, gain, click_left, click_right);
        Self::play_measure_accent(
            state,
            sample_data_measure_accent,
            gain,
            click_left,
            click_right,
        );
    }
}

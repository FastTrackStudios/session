//! Count sample playback
//!
//! Handles playback of count samples (1-8) for count-in patterns.
//! Ported from fts-guide `audio/count_player.rs`.

use crate::samples::AudioSample;

use super::routing::AudioRouter;

/// Count sample player state
#[derive(Debug, Clone)]
pub struct CountPlayerState {
    pub playback_position_count: [usize; 8],
    pub is_playing_count: [bool; 8],
    pub current_count_number: i32,
}

impl CountPlayerState {
    pub fn new() -> Self {
        Self {
            playback_position_count: [0; 8],
            is_playing_count: [false; 8],
            current_count_number: -1,
        }
    }

    /// Reset all playback states
    pub fn reset(&mut self) {
        self.playback_position_count = [0; 8];
        self.is_playing_count = [false; 8];
        self.current_count_number = -1;
    }
}

impl Default for CountPlayerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Count sample player
pub struct CountPlayer;

impl CountPlayer {
    /// Play a specific count sample (0-7 index) and mix to output
    pub fn play_count(
        state: &mut CountPlayerState,
        count_idx: usize,
        sample_data: Option<&AudioSample>,
        gain: f32,
        count_left: &mut f32,
        count_right: &mut f32,
    ) {
        if count_idx >= 8 || !state.is_playing_count[count_idx] {
            return;
        }

        if let Some(decoded_audio) = sample_data {
            if state.playback_position_count[count_idx] < decoded_audio.frames() {
                AudioRouter::mix_decoded_audio(
                    decoded_audio,
                    state.playback_position_count[count_idx],
                    gain,
                    count_left,
                    count_right,
                );
                state.playback_position_count[count_idx] += 1;

                if state.playback_position_count[count_idx] >= decoded_audio.frames() {
                    state.is_playing_count[count_idx] = false;
                    state.playback_position_count[count_idx] = 0;
                }
            } else {
                state.is_playing_count[count_idx] = false;
                state.playback_position_count[count_idx] = 0;
            }
        }
    }

    /// Play all active count samples and mix to output
    pub fn play_all(
        state: &mut CountPlayerState,
        sample_data_count: &[Option<AudioSample>; 8],
        gain: f32,
        count_left: &mut f32,
        count_right: &mut f32,
    ) {
        for count_idx in 0..8 {
            Self::play_count(
                state,
                count_idx,
                sample_data_count[count_idx].as_ref(),
                gain,
                count_left,
                count_right,
            );
        }
    }
}

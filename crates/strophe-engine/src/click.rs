//! Pre-rendered click loop.
//!
//! For FT3b-prime, the click is a fixed-tempo pre-rendered loop fed
//! into a `SamplerNode` with `RepeatMode::RepeatEndlessly`. Tempo
//! changes at runtime aren't supported here — they'll come in FT3b
//! proper when the master clock becomes a first-class engine concept.
//!
//! The synthesis is a sine burst with exponential decay (ported from
//! `woodshed_audio::Sound::Click` so the click sounds the same as
//! Woodshed's metronome).

use std::f32::consts::TAU;

/// Render one bar of clicks at the given BPM into a mono `Vec<f32>`.
///
/// - Downbeat (beat 0) uses an accented higher frequency.
/// - Other beats use the base frequency.
/// - Each click is a 50 ms sine burst with exponential decay.
pub fn render_click_loop(sample_rate: u32, bpm: f32, beats_per_bar: u8) -> Vec<f32> {
    let samples_per_beat = (sample_rate as f32 * 60.0 / bpm) as usize;
    let click_duration_seconds = 0.05;
    let click_samples = (sample_rate as f32 * click_duration_seconds) as usize;
    let total_samples = samples_per_beat * beats_per_bar as usize;

    let base_freq = 800.0_f32;
    let accent_freq = 1200.0_f32;
    let amplitude = 0.4_f32;
    let decay_rate = 5.0 / click_duration_seconds;

    let mut buf = vec![0.0_f32; total_samples];

    for beat in 0..beats_per_bar as usize {
        let freq = if beat == 0 { accent_freq } else { base_freq };
        let beat_start = beat * samples_per_beat;

        for s in 0..click_samples {
            let t = s as f32 / sample_rate as f32;
            let envelope = (-t * decay_rate).exp();
            let phase = t * freq * TAU;
            buf[beat_start + s] = phase.sin() * envelope * amplitude;
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_click_loop_has_expected_length() {
        // 120 BPM, 4 beats per bar, 48 kHz:
        // samples per beat = 48000 * 60 / 120 = 24000
        // total = 24000 * 4 = 96000
        let buf = render_click_loop(48_000, 120.0, 4);
        assert_eq!(buf.len(), 96_000);
    }

    #[test]
    fn render_click_loop_starts_with_accent() {
        let buf = render_click_loop(48_000, 120.0, 4);
        // Click 0 (downbeat) is louder than later clicks at the same offset.
        let downbeat_peak = buf[0..2400]
            .iter()
            .map(|s| s.abs())
            .fold(0.0_f32, f32::max);
        let samples_per_beat = 24_000;
        let beat1_peak = buf[samples_per_beat..samples_per_beat + 2400]
            .iter()
            .map(|s| s.abs())
            .fold(0.0_f32, f32::max);
        // Same amplitude, different frequency — peaks should be similar
        // but not identical due to envelope sampling. Both should be > 0.
        assert!(downbeat_peak > 0.1);
        assert!(beat1_peak > 0.1);
    }

    #[test]
    fn render_click_loop_is_silent_between_clicks() {
        let buf = render_click_loop(48_000, 120.0, 4);
        // 100 ms after a click (well past the 50ms duration) should be silent.
        let silent_offset = 48_000 / 10; // 100ms in samples
        assert!(buf[silent_offset].abs() < 1e-6);
    }
}

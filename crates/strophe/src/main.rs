//! Xilem + Masonry application shell for Strophe.
//!
//! `AppState` owns the authoritative `Session` + `History` + a
//! content-addressed `MediaStore`, alongside the audio `Engine`. The
//! UI is split into *surfaces* (see [`view`]): a persistent transport
//! bar plus one of Tracks / Combination / Settings. View composition
//! lives in `view/`; this file owns `AppState`, the action helpers the
//! views call, and the engine tick/capture glue.
//!
//! The `Engine` is `!Send`; Xilem keeps state on the main thread, so it
//! lives directly in `AppState`. A periodic `task_raw` drives
//! `engine.tick()` (drain input + advance capture + queued layers +
//! flush) and promotes a completed capture into a model `AppendLayer` +
//! engine playback.

mod view;

use std::time::Duration;

use masonry::dpi::LogicalSize;
use masonry::layout::AsUnit;
use masonry_winit::app::{EventLoop, EventLoopBuilder};
use tokio::time;
use winit::error::EventLoopError;
use xilem::core::fork;
use xilem::style::Style;
use xilem::view::{sized_box, task_raw};
use xilem::{WidgetView, WindowOptions, Xilem};

use strophe_engine::media::{InMemoryStore, MediaStore};
use strophe_engine::{CapturePhase, Engine, LayerKey};
use strophe_model::{Edit, History, Layer, Phrase, PlaybackMode, Session, TimeSignature};
use strophe_widgets::theme::{Palette, SP_1, SP_4};
use strophe_widgets::{compute_peaks, Peak};

use view::Surface;

/// Horizontal resolution of the per-track waveform (peak columns).
const WAVEFORM_COLUMNS: usize = 256;
/// Waveform display dimensions.
pub(crate) const WAVEFORM_W: f64 = 240.0;
pub(crate) const WAVEFORM_H: f64 = 40.0;

/// Output-meter bar dimensions + dB floor (shared `meter_view`).
pub(crate) const METER_W: f64 = 240.0;
pub(crate) const METER_H: f64 = 8.0;
pub(crate) const METER_FLOOR_DB: f32 = -60.0;

/// Engine tick cadence (~60 fps). Firewheel wants `update()` roughly
/// every frame; bar-aligned scheduling resolution is bounded by this.
const TICK_INTERVAL: Duration = Duration::from_millis(16);

/// Bars to capture when Record is pressed.
pub(crate) const CAPTURE_BARS: u8 = 1;

pub(crate) struct AppState {
    engine: Result<Engine, String>,
    sample_rate: u32,
    meter_db: [f32; 2],
    capture_phase: CapturePhase,
    /// Active color theme. Dark for now; a light toggle is a later
    /// settings pass (the palette is already fully `palette`-driven).
    palette: Palette,

    // === Model authority ===
    session: Session,
    history: History,
    /// Content-addressed store for captured audio buffers. The model
    /// holds `MediaRef`s; the actual `f32` data lives here.
    store: InMemoryStore,

    // === UI / transport state ===
    /// Which surface is showing.
    surface: Surface,
    /// In a Sum-profile strip, which track (if any) is expanded to show
    /// its per-layer waveforms + mute/gain controls.
    expanded_track: Option<usize>,
    /// Which track index the in-progress capture targets (snapshot of
    /// the armed track at arm time, so re-arming during a capture
    /// doesn't redirect it). The *armed* track itself lives on the
    /// model (`Track.armed`); this is just the in-flight capture's
    /// destination.
    capturing_track: Option<usize>,
    /// Engine layer keys currently looping, so Stop-all can stop them.
    playing: Vec<LayerKey>,
    /// Per-track, per-layer waveform peaks (`[track][layer]`), appended
    /// at capture. The expanded looper strip renders each layer's own
    /// waveform from this.
    layer_peaks: Vec<Vec<Vec<Peak>>>,
    /// Per-track combined (all-layers-summed) waveform peaks, recomputed
    /// at capture. The compact looper strip renders this — it's what the
    /// stacked overdub looks like as one shape.
    combined_peaks: Vec<Vec<Peak>>,
}

impl AppState {
    fn new() -> Self {
        let session = Session::new_default(); // looper profile, 4 tracks
        let n = session.tracks.len();
        let (engine, sample_rate) = match Engine::new() {
            Ok(engine) => {
                let sr = engine.sample_rate();
                (Ok(engine), sr)
            }
            Err(e) => (Err(e.to_string()), 0),
        };
        let mut state = Self {
            engine,
            sample_rate,
            meter_db: [f32::NEG_INFINITY; 2],
            capture_phase: CapturePhase::Idle,
            palette: Palette::dark(),
            session,
            history: History::new(),
            store: InMemoryStore::new(),
            surface: Surface::default(),
            expanded_track: None,
            capturing_track: None,
            playing: Vec::new(),
            layer_peaks: vec![Vec::new(); n],
            combined_peaks: vec![Vec::new(); n],
        };
        state.arm(0); // arm the first track by default
        state
    }

    // === Transport actions ===

    /// Index of the currently-armed track, if any. Derived from the
    /// model (`Track.armed`) — the single source of truth.
    pub(crate) fn armed_track(&self) -> Option<usize> {
        self.session.tracks.iter().position(|t| t.armed)
    }

    /// Arm a track as the capture target. Single-arm semantics: unarm
    /// any other armed track first. Goes through `Edit::ArmTrack`, so
    /// arm state is on the model and survives undo/redo + hand-off.
    pub(crate) fn arm(&mut self, track_idx: usize) {
        let Some(target_id) = self.session.tracks.get(track_idx).map(|t| t.id) else {
            return;
        };
        let stale: Vec<_> = self
            .session
            .tracks
            .iter()
            .filter(|t| t.armed && t.id != target_id)
            .map(|t| t.id)
            .collect();
        for id in stale {
            self.history.commit(
                Edit::ArmTrack {
                    track_id: id,
                    from: true,
                    to: false,
                },
                &mut self.session,
                0,
            );
        }
        if !self.session.tracks[track_idx].armed {
            self.history.commit(
                Edit::ArmTrack {
                    track_id: target_id,
                    from: false,
                    to: true,
                },
                &mut self.session,
                0,
            );
        }
    }

    /// Record into the armed track. No-op if no track is armed.
    ///
    /// - **Master clock on:** a bar-aligned, fixed-length capture with
    ///   count-in (current behavior).
    /// - **Master clock off:** a free / variable-length capture — this
    ///   call *toggles* it: first press starts recording immediately,
    ///   second press stops it (the loop is whatever length you played).
    pub(crate) fn record(&mut self) {
        let Some(armed_idx) = self.armed_track() else {
            return;
        };
        if self.session.master_clock_enabled {
            let count_in = self.session.count_in_bars;
            if let Ok(engine) = &mut self.engine {
                if engine
                    .arm_bar_aligned_capture(CAPTURE_BARS, count_in)
                    .is_ok()
                {
                    self.capturing_track = Some(armed_idx);
                }
            }
        } else {
            let recording = matches!(self.capture_phase, CapturePhase::FreeRecording { .. });
            if let Ok(engine) = &mut self.engine {
                if recording {
                    engine.stop_free_capture(); // tick picks up the Complete buffer
                } else if engine.start_free_capture().is_ok() {
                    self.capturing_track = Some(armed_idx);
                }
            }
        }
    }

    /// Toggle the session master clock and mute/unmute the engine click
    /// to match.
    pub(crate) fn toggle_master_clock(&mut self) {
        let from = self.session.master_clock_enabled;
        let to = !from;
        self.history
            .commit(Edit::SetMasterClock { from, to }, &mut self.session, 0);
        if let Ok(engine) = &mut self.engine {
            engine.set_click_enabled(to);
        }
    }

    /// Nudge the count-in length (clamped to `0..=8` bars).
    pub(crate) fn nudge_count_in(&mut self, delta: i8) {
        let from = self.session.count_in_bars;
        let to = (from as i16 + delta as i16).clamp(0, 8) as u8;
        if to != from {
            self.history
                .commit(Edit::SetCountInBars { from, to }, &mut self.session, 0);
        }
    }

    /// Nudge the session tempo (clamped to `40..=240` BPM) and re-render
    /// the engine click to match.
    pub(crate) fn nudge_bpm(&mut self, delta: f32) {
        let from = self.session.bpm;
        let to = (from + delta).clamp(40.0, 240.0);
        if (to - from).abs() < f32::EPSILON {
            return;
        }
        self.history
            .commit(Edit::SetBpm { from, to }, &mut self.session, 0);
        self.resync_tempo();
    }

    /// Nudge beats-per-bar (the time-signature numerator, clamped to
    /// `1..=16`) and re-render the engine click to match.
    pub(crate) fn nudge_beats(&mut self, delta: i8) {
        let from = self.session.time_signature;
        let num = (from.numerator as i16 + delta as i16).clamp(1, 16) as u8;
        if num == from.numerator {
            return;
        }
        let to = TimeSignature::new(num, from.denominator);
        self.history
            .commit(Edit::SetTimeSignature { from, to }, &mut self.session, 0);
        self.resync_tempo();
    }

    /// Push the session's tempo + bar length to the engine click and
    /// re-apply the master-clock mute state (a re-render starts the new
    /// click node at unity volume).
    ///
    /// **Stops all playing loops first.** They're fixed-length buffers
    /// captured at the old tempo; on the new grid they'd drift against
    /// the click, so a tempo/bar-length change clears them rather than
    /// letting them run out of phase.
    fn resync_tempo(&mut self) {
        self.stop_all();
        let bpm = self.session.bpm;
        let beats = self.session.time_signature.numerator;
        let clock = self.session.master_clock_enabled;
        if let Ok(engine) = &mut self.engine {
            let _ = engine.set_tempo(bpm, beats);
            engine.set_click_enabled(clock);
        }
    }

    /// Recompute a track's combined (all-layers-summed) waveform peaks
    /// from the content store. Called at capture time, when a layer is
    /// added. Sums every layer regardless of mute — the compact strip
    /// shows the full stack as one shape.
    fn recompute_combined(&mut self, track_idx: usize) {
        let mut sum: Vec<f32> = Vec::new();
        if let Some(track) = self.session.tracks.get(track_idx) {
            for layer in &track.layers {
                if let Some(phrase) = self.session.phrases.get(&layer.phrase_id) {
                    if let Some(buf) = self.store.get(&phrase.media) {
                        if buf.samples.len() > sum.len() {
                            sum.resize(buf.samples.len(), 0.0);
                        }
                        for (i, s) in buf.samples.iter().enumerate() {
                            sum[i] += *s;
                        }
                    }
                }
            }
        }
        if let Some(slot) = self.combined_peaks.get_mut(track_idx) {
            *slot = compute_peaks(&sum, WAVEFORM_COLUMNS);
        }
    }

    /// Stop every looping voice.
    pub(crate) fn stop_all(&mut self) {
        let keys: Vec<LayerKey> = self.playing.drain(..).collect();
        if let Ok(engine) = &mut self.engine {
            for key in keys {
                engine.stop_layer(key);
            }
        }
    }

    // === Layer / variation actions ===

    /// Stop one layer's voice and forget it.
    fn stop_layer_key(&mut self, key: LayerKey) {
        if let Ok(engine) = &mut self.engine {
            engine.stop_layer(key);
        }
        self.playing.retain(|k| *k != key);
    }

    /// (Re)play a layer from the content store at the next bar boundary,
    /// at its stored gain. No-op if the layer or its media is missing.
    fn play_layer_from_store(&mut self, track_idx: usize, layer_idx: usize) {
        let Some(track) = self.session.tracks.get(track_idx) else {
            return;
        };
        let Some(layer) = track.layers.get(layer_idx) else {
            return;
        };
        let Some(phrase) = self.session.phrases.get(&layer.phrase_id) else {
            return;
        };
        let Some(buf) = self.store.get(&phrase.media) else {
            return;
        };
        let samples = buf.samples.clone();
        let gain = layer.gain;
        let key = LayerKey::new(track.id, layer_idx as u16);
        if let Ok(engine) = &mut self.engine {
            let _ = engine.play_layer_at_next_bar(key, samples, gain, true);
        }
        if !self.playing.contains(&key) {
            self.playing.push(key);
        }
    }

    /// Pick the active variation on a `SelectOne` track (Deeler): stop
    /// the previously-active layer, commit the model edit, play the new
    /// one at the next bar.
    pub(crate) fn select_variation(&mut self, track_idx: usize, layer_idx: u16) {
        let Some(track) = self.session.tracks.get(track_idx) else {
            return;
        };
        let track_id = track.id;
        let from = track.playback_mode.active_layer();
        if let Some(active) = from {
            self.stop_layer_key(LayerKey::new(track_id, active));
        }
        self.history.commit(
            Edit::SelectActiveLayer {
                track_id,
                from,
                to: Some(layer_idx),
            },
            &mut self.session,
            0,
        );
        self.play_layer_from_store(track_idx, layer_idx as usize);
    }

    /// Toggle a layer's mute (Sum profile). Mute stops its voice; unmute
    /// replays it from the store.
    pub(crate) fn toggle_layer_mute(&mut self, track_idx: usize, layer_idx: u16) {
        let Some(track) = self.session.tracks.get(track_idx) else {
            return;
        };
        let track_id = track.id;
        let Some(layer) = track.layers.get(layer_idx as usize) else {
            return;
        };
        let from = layer.muted;
        let to = !from;
        self.history.commit(
            Edit::SetLayerMute {
                track_id,
                layer_index: layer_idx,
                from,
                to,
            },
            &mut self.session,
            0,
        );
        let key = LayerKey::new(track_id, layer_idx);
        if to {
            self.stop_layer_key(key);
        } else {
            self.play_layer_from_store(track_idx, layer_idx as usize);
        }
    }

    /// Nudge a layer's gain by `delta` (clamped to `0.0..=2.0`) and apply
    /// it live to the playing voice.
    pub(crate) fn nudge_layer_gain(&mut self, track_idx: usize, layer_idx: u16, delta: f32) {
        let Some(track) = self.session.tracks.get(track_idx) else {
            return;
        };
        let track_id = track.id;
        let Some(layer) = track.layers.get(layer_idx as usize) else {
            return;
        };
        let from = layer.gain;
        let to = (from + delta).clamp(0.0, 2.0);
        self.history.commit(
            Edit::SetLayerGain {
                track_id,
                layer_index: layer_idx,
                from,
                to,
            },
            &mut self.session,
            0,
        );
        let key = LayerKey::new(track_id, layer_idx);
        if let Ok(engine) = &mut self.engine {
            engine.set_layer_gain(key, to);
        }
    }

    /// Expand/collapse a Sum-profile track's per-layer controls.
    pub(crate) fn toggle_expand(&mut self, track_idx: usize) {
        self.expanded_track = if self.expanded_track == Some(track_idx) {
            None
        } else {
            Some(track_idx)
        };
    }

    // === Session-level actions ===

    /// Switch the whole session between the looper-pedal and Deeler
    /// profiles. Stops all voices and starts a fresh session — this is
    /// a destructive "new project in profile X," not a per-track mode
    /// change (that's `Edit::SetTrackPlaybackMode`).
    pub(crate) fn switch_profile(&mut self, deeler: bool) {
        self.stop_all();
        self.session = if deeler {
            Session::new_deeler_profile()
        } else {
            Session::new_default()
        };
        self.history = History::new();
        self.store = InMemoryStore::new();
        self.expanded_track = None;
        self.capturing_track = None;
        let n = self.session.tracks.len();
        self.layer_peaks = vec![Vec::new(); n];
        self.combined_peaks = vec![Vec::new(); n];
        self.surface = Surface::Tracks;
        self.arm(0); // arm the first track of the new session
        // New session resets tempo + master-clock to defaults; push them
        // to the engine click (re-renders + re-applies mute state).
        self.resync_tempo();
    }

    /// True if the session is in the Deeler (SelectOne) profile.
    pub(crate) fn is_deeler(&self) -> bool {
        matches!(self.session.default_playback_mode, PlaybackMode::SelectOne { .. })
    }

    pub(crate) fn show(&mut self, surface: Surface) {
        self.surface = surface;
    }
}

/// Format a dB value for the meter readout (right-aligned, `-inf` for
/// silence).
pub(crate) fn fmt_db(v: f32) -> String {
    if v == f32::NEG_INFINITY {
        "  -inf".to_string()
    } else {
        format!("{v:>6.1}")
    }
}

/// Human-readable capture-phase line for the transport.
pub(crate) fn capture_phase_text(phase: &CapturePhase, sample_rate: u32) -> String {
    match phase {
        CapturePhase::Idle => "ready".to_string(),
        CapturePhase::Waiting {
            bars_remaining,
            samples_until_next_bar,
        } => {
            let ms = if sample_rate > 0 {
                samples_until_next_bar * 1000 / sample_rate as usize
            } else {
                0
            };
            format!("count-in: {bars_remaining} bar(s) left · next bar in {ms} ms")
        }
        CapturePhase::Recording { progress } => {
            format!("recording… {:.0}%", progress * 100.0)
        }
        CapturePhase::FreeRecording { samples_done } => {
            let secs = if sample_rate > 0 {
                *samples_done as f32 / sample_rate as f32
            } else {
                0.0
            };
            format!("recording (free)… {secs:.1}s — press Record to stop")
        }
        CapturePhase::Complete => "captured".to_string(),
    }
}

fn app_logic(state: &mut AppState) -> impl WidgetView<AppState> + use<> {
    let palette = state.palette;

    let tick_task = task_raw(
        move |proxy, _| async move {
            let mut interval = time::interval(TICK_INTERVAL);
            interval.tick().await; // first tick immediate; skip
            loop {
                interval.tick().await;
                if proxy.message(()).is_err() {
                    break;
                }
            }
        },
        |state: &mut AppState, _: ()| {
            // Advance the engine and read back meter / phase / any
            // completed capture, all within one engine borrow.
            let (meter, phase, captured) = match &mut state.engine {
                Ok(engine) => {
                    let _ = engine.tick();
                    (
                        engine.peak_db(),
                        engine.pending_capture_progress(),
                        engine.take_bar_aligned_capture(),
                    )
                }
                Err(_) => (state.meter_db, state.capture_phase.clone(), None),
            };
            state.meter_db = meter;
            state.capture_phase = phase;

            // Promote a completed capture into a model layer + engine
            // playback.
            if let Some(samples) = captured {
                let target = state.capturing_track.take();
                if let Some(track_idx) = target {
                    // A free capture stopped instantly yields an empty
                    // buffer — skip rather than store a zero-length layer.
                    if !samples.is_empty() && track_idx < state.session.tracks.len() {
                        let sr = state.sample_rate;
                        let bars = state.session.bars_per_phrase;
                        let bpm = state.session.bpm;
                        let media_ref = state.store.put(&samples, sr);
                        // Per-layer peaks (the expanded strip renders
                        // each), computed before `samples` moves into the
                        // engine.
                        let layer_pk = compute_peaks(&samples, WAVEFORM_COLUMNS);
                        if let Some(track_slot) = state.layer_peaks.get_mut(track_idx) {
                            track_slot.push(layer_pk);
                        }
                        let phrase = Phrase::new(media_ref, bars, bpm, 0);
                        let layer = Layer::new(phrase.id);
                        let track_id = state.session.tracks[track_idx].id;
                        let layer_index = state.session.tracks[track_idx].layers.len() as u16;
                        state.history.commit(
                            Edit::AppendLayer {
                                track_id,
                                phrase,
                                layer,
                            },
                            &mut state.session,
                            0,
                        );
                        // Combined (summed) peaks for the compact strip,
                        // now that the new layer is in the store + model.
                        state.recompute_combined(track_idx);
                        let key = LayerKey::new(track_id, layer_index);
                        // Clocked: start at the next bar (phase-lock).
                        // Unclocked/free: start immediately (no grid to
                        // wait for).
                        let clocked = state.session.master_clock_enabled;
                        if let Ok(engine) = &mut state.engine {
                            let _ = if clocked {
                                engine.play_layer_at_next_bar(key, samples, 1.0, true)
                            } else {
                                engine.play_layer(key, samples, 1.0, true)
                            };
                        }
                        state.playing.push(key);
                    }
                }
            }
        },
    );

    fork(
        sized_box(view::app_shell(state))
            .padding(SP_4)
            .background_color(palette.bg),
        tick_task,
    )
}

/// Overlay palette colors onto masonry's built-in default property set.
/// Masonry's defaults hardcode near-white text + dark button surfaces
/// (a dark-theme assumption), so a bare `label(...)` ignores our palette
/// without this. Set once at startup; a mid-session theme switch would
/// need a property-set swap (out of scope until the settings pass).
fn build_default_properties(palette: &Palette) -> masonry::core::DefaultProperties {
    use masonry::core::DefaultProperties;
    use masonry::properties::{Background, BorderColor, BorderWidth, ContentColor, CornerRadius};
    use masonry::widgets::{Button, Label};

    let mut properties: DefaultProperties = masonry::theme::default_property_set();

    properties.insert::<Label, _>(ContentColor::new(palette.text));

    properties.insert::<Button, _>(Background::Color(palette.surface_2));
    properties.insert::<Button, _>(BorderColor {
        color: palette.surface_hover,
    });
    properties.insert::<Button, _>(BorderWidth { width: SP_1 });
    properties.insert::<Button, _>(CornerRadius { radius: 6.px() });

    properties
}

pub fn run(event_loop: EventLoopBuilder) -> Result<(), EventLoopError> {
    let state = AppState::new();
    let default_properties = build_default_properties(&state.palette);
    let window_options = WindowOptions::new("Strophe")
        .with_min_inner_size(LogicalSize::new(480.0, 360.0))
        .with_initial_inner_size(LogicalSize::new(720.0, 540.0));
    let app = Xilem::new_simple(state, app_logic, window_options)
        .with_default_properties(default_properties);
    app.run_in(event_loop)?;
    Ok(())
}

fn main() -> Result<(), EventLoopError> {
    run(EventLoop::with_user_event())
}

//! Xilem + Masonry application shell for Strophe (FT4).
//!
//! FT4.2: the model ↔ engine ↔ UI junction. `AppState` now owns the
//! authoritative `Session` + `History` + a content-addressed
//! `MediaStore`, alongside the audio `Engine`. Track strips render
//! from `session.tracks`; one track is "armed" for recording. Record
//! captures into the armed track: the buffer is stored
//! (content-addressed → `MediaRef`), wrapped in a `Phrase` + `Layer`,
//! committed to history via `Edit::AppendLayer`, and queued to play
//! in the engine at the next bar boundary. Multiple tracks loop
//! simultaneously (looper-pedal `Sum` profile).
//!
//! The `Engine` is `!Send`; Xilem keeps state on the main thread, so
//! it lives directly in `AppState`. `tick()` (driven by a periodic
//! `task_raw`) advances the whole engine — drains input, advances
//! capture + queued layers, flushes Firewheel — and the tick handler
//! promotes a completed capture into a model `AppendLayer` + engine
//! `play_layer_at_next_bar`.

use std::time::Duration;

use masonry::dpi::LogicalSize;
use masonry::layout::AsUnit;
use masonry::peniko::Color;
use masonry_winit::app::{EventLoop, EventLoopBuilder};
use tokio::time;
use winit::error::EventLoopError;
use xilem::core::fork;
use xilem::style::Style;
use xilem::view::{
    flex_col, flex_row, label, sized_box, task_raw, text_button, AnyFlexChild, FlexExt,
};
use xilem::{WidgetView, WindowOptions, Xilem};

use strophe_engine::media::{InMemoryStore, MediaStore};
use strophe_engine::{CapturePhase, Engine, LayerKey};
use strophe_model::{Edit, History, Layer, Phrase, Session};
use strophe_widgets::theme::{mono_family, Palette, SP_1, SP_2, SP_3, SP_4, TS_SM, TS_XL, TS_XS};
use strophe_widgets::{compute_peaks, waveform_view, Peak};

/// Horizontal resolution of the per-track waveform (peak columns).
const WAVEFORM_COLUMNS: usize = 256;
/// Waveform display dimensions.
const WAVEFORM_W: f64 = 240.0;
const WAVEFORM_H: f64 = 40.0;

/// Engine tick cadence (~60 fps). Firewheel wants `update()` roughly
/// every frame; bar-aligned scheduling resolution is bounded by this.
const TICK_INTERVAL: Duration = Duration::from_millis(16);

/// Bars to capture when Record is pressed.
const CAPTURE_BARS: u8 = 1;
/// Bars of click count-in before capture begins.
const COUNT_IN_BARS: u8 = 1;

struct AppState {
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
    /// Index into `session.tracks` that Record captures into.
    armed_track: usize,
    /// Which track index the in-progress capture targets (snapshot of
    /// `armed_track` at arm time, so changing the armed track during
    /// a capture doesn't redirect it).
    capturing_track: Option<usize>,
    /// Engine layer keys currently looping, so Stop-all can stop them.
    playing: Vec<LayerKey>,
    /// Per-track waveform peaks (the most recently captured layer's
    /// peaks), indexed parallel to `session.tracks`. Computed once at
    /// capture time, not per frame.
    track_peaks: Vec<Vec<Peak>>,
}

impl AppState {
    fn new() -> Self {
        let session = Session::new_default(); // looper profile, 4 tracks
        let track_peaks = vec![Vec::new(); session.tracks.len()];
        let (engine, sample_rate) = match Engine::new() {
            Ok(engine) => {
                let sr = engine.sample_rate();
                (Ok(engine), sr)
            }
            Err(e) => (Err(e.to_string()), 0),
        };
        Self {
            engine,
            sample_rate,
            meter_db: [f32::NEG_INFINITY; 2],
            capture_phase: CapturePhase::Idle,
            palette: Palette::dark(),
            session,
            history: History::new(),
            store: InMemoryStore::new(),
            armed_track: 0,
            capturing_track: None,
            playing: Vec::new(),
            track_peaks,
        }
    }
}

fn fmt_db(v: f32) -> String {
    if v == f32::NEG_INFINITY {
        "  -inf".to_string()
    } else {
        format!("{v:>6.1}")
    }
}

fn capture_phase_text(phase: &CapturePhase, sample_rate: u32) -> String {
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
        CapturePhase::Complete => "captured".to_string(),
    }
}

fn app_logic(state: &mut AppState) -> impl WidgetView<AppState> + use<> {
    let status = match &state.engine {
        Ok(_) => format!("engine running @ {} Hz · click looping", state.sample_rate),
        Err(e) => format!("engine unavailable: {e}"),
    };

    let [l, r] = state.meter_db;
    let meter_line = format!("output peak   L {} dB   R {} dB", fmt_db(l), fmt_db(r));
    let capture_line = capture_phase_text(&state.capture_phase, state.sample_rate);
    let loops_line = format!("{} loop(s) playing", state.playing.len());

    // Per-track rows: [arm button] [waveform] [layer count].
    let armed = state.armed_track;
    let palette = state.palette;
    let track_rows: Vec<AnyFlexChild<AppState>> = state
        .session
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let marker = if i == armed { "●" } else { "○" };
            let arm_label = format!("{}  {}", marker, track.name);
            // Waveform fill is the track's own color (product-specific,
            // not a theme token); the zero-line reads from the shared
            // palette so it tracks the active theme.
            let wave_color = Color::from_rgb8(track.color.r, track.color.g, track.color.b);
            let zero_color = palette.text_disabled;
            let peaks = state.track_peaks.get(i).cloned().unwrap_or_default();
            let count_label = format!("{} layer(s)", track.layers.len());
            flex_row((
                text_button(arm_label, move |s: &mut AppState| {
                    s.armed_track = i;
                }),
                sized_box(waveform_view(peaks, wave_color, zero_color))
                    .width(WAVEFORM_W.px())
                    .height(WAVEFORM_H.px()),
                label(count_label).text_size(TS_XS),
            ))
            .gap(SP_3)
            .into_any_flex()
        })
        .collect();

    let controls = flex_row((
        text_button("● Record (armed track)", |state: &mut AppState| {
            if let Ok(engine) = &mut state.engine {
                if engine
                    .arm_bar_aligned_capture(CAPTURE_BARS, COUNT_IN_BARS)
                    .is_ok()
                {
                    state.capturing_track = Some(state.armed_track);
                }
            }
        }),
        text_button("■ Stop all loops", |state: &mut AppState| {
            let keys: Vec<LayerKey> = state.playing.drain(..).collect();
            if let Ok(engine) = &mut state.engine {
                for key in keys {
                    engine.stop_layer(key);
                }
            }
        }),
    ))
    .gap(SP_3);

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
                if let Some(track_idx) = state.capturing_track.take() {
                    if track_idx < state.session.tracks.len() {
                        let sr = state.sample_rate;
                        let bars = state.session.bars_per_phrase;
                        let bpm = state.session.bpm;
                        let media_ref = state.store.put(&samples, sr);
                        // Compute the waveform peaks before `samples`
                        // is moved into the engine.
                        state.track_peaks[track_idx] =
                            compute_peaks(&samples, WAVEFORM_COLUMNS);
                        let phrase = Phrase::new(media_ref, bars, bpm, 0);
                        let layer = Layer::new(phrase.id);
                        let track_id = state.session.tracks[track_idx].id;
                        let layer_index =
                            state.session.tracks[track_idx].layers.len() as u16;
                        state.history.commit(
                            Edit::AppendLayer {
                                track_id,
                                phrase,
                                layer,
                            },
                            &mut state.session,
                            0,
                        );
                        let key = LayerKey::new(track_id, layer_index);
                        if let Ok(engine) = &mut state.engine {
                            let _ = engine
                                .play_layer_at_next_bar(key, samples, 1.0, true);
                        }
                        state.playing.push(key);
                    }
                }
            }
        },
    );

    fork(
        sized_box(
            flex_col((
                label("Strophe").text_size(TS_XL),
                label(status).text_size(TS_SM),
                // Live numeric readouts in mono so digit-width jitter
                // doesn't shuffle the layout every tick.
                label(meter_line).text_size(TS_SM).font(mono_family()),
                label(capture_line).text_size(TS_SM).font(mono_family()),
                label(loops_line).text_size(TS_SM).font(mono_family()),
                label("Tracks (click to arm):").text_size(TS_XS),
                flex_col(track_rows).gap(SP_2),
                controls,
            ))
            .gap(SP_2),
        )
        .padding(SP_4)
        .background_color(palette.bg),
        tick_task,
    )
}

/// Overlay palette colors onto masonry's built-in default property
/// set. Masonry's defaults hardcode near-white text + dark button
/// surfaces (a dark-theme assumption), so a bare `label(...)` ignores
/// our palette without this. Set once at startup; a mid-session theme
/// switch would need a property-set swap (out of scope until the
/// settings pass).
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

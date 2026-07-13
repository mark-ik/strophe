//! The app state behind the genet UI: the real `strophe_model::Session` +
//! `History`, plus the audio `strophe_engine::Engine` and its content store.
//!
//! Every data-bearing gesture commits a real `Edit`, so undo/redo + the future
//! sync layer see exactly what the UI did; the engine-driving methods mirror the
//! host's (`record` / `arm` / `stop_all` / `play_layer_from_store` / …)
//! so audio behaviour remains independent of the UI framework. The `Engine` is
//! `!Send`; winit runs the app on the main thread, so it lives directly here and
//! is advanced by [`tick`](AppState::tick) from the host's ~60fps timer.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use armillary::ActorHandle;
use audio_primitives::{MeterBallistics, PeakMeterSmoother, WaveformPeak};
use strophe_engine::export::ExportLength;
use strophe_engine::media::{InMemoryStore, MediaStore};
use strophe_engine::{
    AudioDeviceSelection, AudioDevices, CapturePhase, Engine, LayerKey, available_audio_devices,
};
use strophe_model::{
    Edit, History, Layer, MediaRef, Phrase, ProjectBundle, Session, Track, TrackColor, TrackId,
};
use xilem_serval::SelectState;

use crate::identity::LocalIdentity;
use crate::project_io::{ProjectCommand, ProjectUpdate};

/// Meter dB floor for the 0..1 level the output meters display.
const METER_FLOOR_DB: f32 = -60.0;

fn normalize_meter_db(db: f32) -> f32 {
    if !db.is_finite() || db <= METER_FLOOR_DB {
        0.0
    } else {
        ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
    }
}

enum ProjectStatus {
    Idle,
    Saving,
    Loading,
    Exporting,
    Saved,
    Exported(PathBuf),
    Error(String),
}

pub struct AppState {
    pub session: Session,
    pub history: History,

    // === audio ===
    engine: Result<Engine, String>,
    sample_rate: u32,
    store: InMemoryStore,
    capture_phase: CapturePhase,
    /// The track the in-flight capture targets (snapshot at Record time, so
    /// re-arming mid-capture does not redirect it).
    capturing_track: Option<usize>,
    /// Engine layer keys currently looping, so stop-all / mute can stop them.
    playing: Vec<LayerKey>,
    /// Latest output peak, dB per channel, read back each tick.
    meter_db: [f32; 2],
    /// Host-local display smoothing; timing is UI policy, not session state.
    meter_display: [PeakMeterSmoother; 2],
    last_meter_tick: Instant,
    /// Blobs referenced by an opened project but unavailable locally. Their
    /// layers remain in the model and stay silent until the blobs arrive.
    pub missing_media: BTreeSet<MediaRef>,
    project_path: Option<PathBuf>,
    saved_head: strophe_model::NodeId,
    project_status: ProjectStatus,
    project_worker: ActorHandle<ProjectCommand>,
    /// Durable host identity. Its secret and unlock state never enter a project.
    identity: Result<LocalIdentity, String>,
    /// Host-local export intent. It changes only the rendered file, never the
    /// project graph or its syncable history.
    export_length: ExportLength,
    /// Per-launch device catalog and selections. Device IDs belong to the host,
    /// not to a project that might travel to another machine.
    audio_devices: AudioDevices,
    pub(crate) audio_input_select: SelectState,
    pub(crate) audio_output_select: SelectState,
    applied_audio_input: usize,
    applied_audio_output: usize,
    audio_status: String,

    // === app-local UI (graduation notes) ===
    /// Audible metronome. Drives the engine click directly; distinct from the
    /// master clock (which governs bar-aligned capture + count-in).
    pub click: bool,
    /// Solo set. App-local: the model has no solo yet (a mix-bus concern).
    pub solo: BTreeSet<TrackId>,
}

impl AppState {
    /// A new, empty looper-pedal session. Captures append real playable layers.
    pub fn new(
        project_worker: ActorHandle<ProjectCommand>,
        identity: Result<LocalIdentity, String>,
    ) -> Self {
        Self::from_project_parts(
            Session::new_default(),
            History::new(),
            InMemoryStore::new(),
            BTreeSet::new(),
            project_worker,
            identity,
        )
    }

    fn from_project_parts(
        session: Session,
        history: History,
        store: InMemoryStore,
        missing_media: BTreeSet<MediaRef>,
        project_worker: ActorHandle<ProjectCommand>,
        identity: Result<LocalIdentity, String>,
    ) -> Self {
        let audio_devices = available_audio_devices();
        let (engine, sample_rate, audio_status) = match Engine::new() {
            Ok(e) => {
                let sr = e.sample_rate();
                (Ok(e), sr, "system audio".to_string())
            }
            Err(e) => (Err(e.to_string()), 0, "audio unavailable".to_string()),
        };
        let saved_head = history.head;
        let mut state = Self {
            session,
            history,
            engine,
            sample_rate,
            store,
            capture_phase: CapturePhase::Idle,
            capturing_track: None,
            playing: Vec::new(),
            meter_db: [f32::NEG_INFINITY; 2],
            meter_display: [PeakMeterSmoother::default(), PeakMeterSmoother::default()],
            last_meter_tick: Instant::now(),
            missing_media,
            project_path: None,
            saved_head,
            project_status: ProjectStatus::Idle,
            project_worker,
            identity,
            export_length: ExportLength::OneCycle,
            audio_devices,
            audio_input_select: SelectState::default(),
            audio_output_select: SelectState::default(),
            applied_audio_input: 0,
            applied_audio_output: 0,
            audio_status,
            click: true,
            solo: BTreeSet::new(),
        };
        state.set_meter_ballistics(MeterBallistics::default());
        // Apply the initial click + tempo to the engine.
        state.resync_tempo();
        state
    }

    pub fn project_label(&self) -> String {
        self.project_path
            .as_deref()
            .and_then(|path| path.file_stem())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string())
    }

    pub fn project_status_label(&self) -> String {
        match &self.project_status {
            ProjectStatus::Saving => "saving".to_string(),
            ProjectStatus::Loading => "opening".to_string(),
            ProjectStatus::Exporting => "exporting mix".to_string(),
            ProjectStatus::Saved => {
                if self.is_dirty() {
                    "unsaved changes".to_string()
                } else {
                    "saved".to_string()
                }
            }
            ProjectStatus::Exported(path) => format!(
                "exported {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
            ProjectStatus::Error(message) => format!("project error: {message}"),
            ProjectStatus::Idle => {
                if self.is_dirty() {
                    "unsaved changes".to_string()
                } else {
                    "new session".to_string()
                }
            }
        }
    }

    pub fn identity_status_label(&self) -> String {
        match &self.identity {
            Ok(identity) => format!("local session · {}", identity.fingerprint()),
            Err(_) => "local session · identity unavailable".to_string(),
        }
    }

    pub fn is_project_io_active(&self) -> bool {
        matches!(
            self.project_status,
            ProjectStatus::Saving | ProjectStatus::Loading | ProjectStatus::Exporting
        )
    }

    pub fn choose_project_to_open(&mut self) {
        if self.is_project_io_active() {
            return;
        }
        if self.is_recording() {
            self.project_status = ProjectStatus::Error("stop recording before opening".to_string());
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Strophe project", &["strophe"])
            .pick_file()
        {
            self.project_status = ProjectStatus::Loading;
            if !self.project_worker.command(ProjectCommand::Open { path }) {
                self.project_status = ProjectStatus::Error("project worker stopped".to_string());
            }
        }
    }

    pub fn choose_project_to_save(&mut self) {
        if self.is_project_io_active() {
            return;
        }
        if self.is_recording() {
            self.project_status = ProjectStatus::Error("stop recording before saving".to_string());
            return;
        }
        let path = match self.project_path.clone() {
            Some(path) => Some(path),
            None => rfd::FileDialog::new()
                .add_filter("Strophe project", &["strophe"])
                .set_file_name("untitled.strophe")
                .save_file()
                .map(|path| ensure_extension(path, "strophe")),
        };
        if let Some(path) = path {
            let bundle = ProjectBundle::new(self.session.clone(), self.history.clone());
            self.project_status = ProjectStatus::Saving;
            if !self.project_worker.command(ProjectCommand::Save {
                path,
                bundle,
                media: self.store.clone(),
                saved_head: self.history.head,
            }) {
                self.project_status = ProjectStatus::Error("project worker stopped".to_string());
            }
        }
    }

    pub fn choose_mix_export(&mut self) {
        if self.is_project_io_active() {
            return;
        }
        if self.is_recording() {
            self.project_status =
                ProjectStatus::Error("stop recording before exporting".to_string());
            return;
        }
        let file_name = format!("{}-mix.wav", self.project_label());
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV audio", &["wav"])
            .set_file_name(file_name)
            .save_file()
            .map(|path| ensure_extension(path, "wav"))
        {
            self.project_status = ProjectStatus::Exporting;
            if !self.project_worker.command(ProjectCommand::ExportMix {
                path,
                session: self.session.clone(),
                media: self.store.clone(),
                solo: self.solo.clone(),
                length: self.export_length,
            }) {
                self.project_status = ProjectStatus::Error("project worker stopped".to_string());
            }
        }
    }

    pub fn export_uses_bars(&self) -> bool {
        matches!(self.export_length, ExportLength::Bars(_))
    }

    pub fn export_bars(&self) -> Option<u8> {
        match self.export_length {
            ExportLength::Bars(bars) => Some(bars),
            ExportLength::OneCycle => None,
        }
    }

    pub fn export_one_cycle(&mut self) {
        self.export_length = ExportLength::OneCycle;
    }

    pub fn export_session_bars(&mut self) {
        self.export_length = ExportLength::Bars(self.session.bars_per_phrase.max(1));
    }

    pub fn adjust_export_bars(&mut self, delta: i8) {
        let bars = self
            .export_bars()
            .unwrap_or_else(|| self.session.bars_per_phrase.max(1));
        self.export_length = ExportLength::Bars(if delta.is_negative() {
            bars.saturating_sub(delta.unsigned_abs()).max(1)
        } else {
            bars.saturating_add(delta as u8)
        });
    }

    pub fn apply_project_update(&mut self, update: ProjectUpdate) {
        match update {
            ProjectUpdate::Saved { path, saved_head } => {
                self.project_path = Some(path);
                self.saved_head = saved_head;
                self.project_status = ProjectStatus::Saved;
            }
            ProjectUpdate::Opened { path, loaded } => {
                self.stop_all();
                self.session = loaded.bundle.session;
                self.history = loaded.bundle.history;
                self.store = loaded.media;
                self.missing_media = loaded.missing_media;
                self.capture_phase = CapturePhase::Idle;
                self.capturing_track = None;
                self.solo.clear();
                self.project_path = Some(path);
                self.saved_head = self.history.head;
                self.project_status = ProjectStatus::Saved;
                self.resync_tempo();
                self.reconcile_all_playback();
            }
            ProjectUpdate::Exported { path } => {
                self.project_status = ProjectStatus::Exported(path);
            }
            ProjectUpdate::Failed { action, message } => {
                self.project_status = ProjectStatus::Error(format!("{action}: {message}"));
            }
        }
    }

    fn is_dirty(&self) -> bool {
        self.history.head != self.saved_head
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    // === derived state the view reads ===

    pub fn armed_index(&self) -> Option<usize> {
        self.session.tracks.iter().position(|t| t.armed)
    }

    /// Whether a capture is in progress (drives the record light + rail).
    pub fn is_recording(&self) -> bool {
        matches!(
            self.capture_phase,
            CapturePhase::Waiting { .. }
                | CapturePhase::Recording { .. }
                | CapturePhase::FreeRecording { .. }
        )
    }

    /// The output meter level `0..1` for channel `ch` (0 = L, 1 = R), mapped
    /// from the read-back dB against the display floor.
    pub fn meter_level(&self, ch: usize) -> f32 {
        self.meter_display
            .get(ch)
            .map(|meter| meter.reading().level)
            .unwrap_or(0.0)
    }

    pub fn meter_peak_level(&self, ch: usize) -> f32 {
        self.meter_display
            .get(ch)
            .map(|meter| meter.reading().peak)
            .unwrap_or(0.0)
    }

    /// Latest output level in dB, for the textual peak readout.
    pub fn meter_db(&self, ch: usize) -> f32 {
        self.meter_db.get(ch).copied().unwrap_or(f32::NEG_INFINITY)
    }

    /// Host-local meter timing. A settings surface can replace this without
    /// mutating project or collaboration state.
    pub fn set_meter_ballistics(&mut self, ballistics: MeterBallistics) {
        for meter in &mut self.meter_display {
            meter.set_ballistics(ballistics);
        }
    }

    pub(crate) fn render_track_waveform(
        &self,
        track_index: usize,
        columns: usize,
    ) -> Option<Vec<WaveformPeak>> {
        strophe_engine::waveform::render_track_peaks(
            &self.session,
            &self.store,
            track_index,
            columns,
        )
        .ok()
    }

    pub(crate) fn render_layer_waveform(
        &self,
        track_index: usize,
        layer_index: usize,
        columns: usize,
    ) -> Option<Vec<WaveformPeak>> {
        strophe_engine::waveform::render_layer_peaks(
            &self.session,
            &self.store,
            track_index,
            layer_index,
            columns,
        )
        .ok()
    }

    pub fn layer_waveform_available(&self, track_index: usize, layer_index: usize) -> bool {
        self.session
            .tracks
            .get(track_index)
            .and_then(|track| track.layers.get(layer_index))
            .and_then(|layer| self.session.phrases.get(&layer.phrase_id))
            .and_then(|phrase| self.store.get(&phrase.media))
            .is_some_and(|buffer| !buffer.samples.is_empty())
    }

    pub fn track_has_audible_layers(&self, track_index: usize) -> bool {
        self.session.tracks.get(track_index).is_some_and(|track| {
            track.layers.iter().enumerate().any(|(index, layer)| {
                track
                    .playback_mode
                    .is_layer_audible(index as u16, layer.muted)
            })
        })
    }

    pub fn track_waveform_available(&self, track_index: usize) -> bool {
        let Some(track) = self.session.tracks.get(track_index) else {
            return false;
        };
        let mut sample_rate = None;
        let mut found = false;
        for (index, layer) in track.layers.iter().enumerate() {
            if !track
                .playback_mode
                .is_layer_audible(index as u16, layer.muted)
            {
                continue;
            }
            let Some(buffer) = self
                .session
                .phrases
                .get(&layer.phrase_id)
                .and_then(|phrase| self.store.get(&phrase.media))
            else {
                return false;
            };
            if buffer.samples.is_empty() {
                return false;
            }
            match sample_rate {
                Some(expected) if expected != buffer.sample_rate => return false,
                None => sample_rate = Some(buffer.sample_rate),
                _ => {}
            }
            found = true;
        }
        found
    }

    pub fn input_device_options(&self) -> Vec<String> {
        audio_device_options(&self.audio_devices.inputs)
    }

    pub fn output_device_options(&self) -> Vec<String> {
        audio_device_options(&self.audio_devices.outputs)
    }

    pub fn audio_status_label(&self) -> &str {
        &self.audio_status
    }

    // === the engine tick (host calls ~60fps via runner.update) ===

    /// Advance the engine, read back the meter + capture phase, and promote a
    /// completed capture into a model layer + looping playback.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let meter_delta = now
            .duration_since(self.last_meter_tick)
            .as_secs_f32()
            .clamp(0.0, 0.25);
        self.last_meter_tick = now;
        self.apply_audio_device_selection();
        let (meter, phase, captured) = match &mut self.engine {
            Ok(engine) => {
                let _ = engine.tick();
                (
                    engine.peak_db(),
                    engine.pending_capture_progress(),
                    engine.take_bar_aligned_capture(),
                )
            }
            Err(_) => ([f32::NEG_INFINITY; 2], self.capture_phase.clone(), None),
        };
        self.meter_db = meter;
        for (channel, smoother) in self.meter_display.iter_mut().enumerate() {
            smoother.update(normalize_meter_db(meter[channel]), meter_delta);
        }
        self.capture_phase = phase;

        if let Some(samples) = captured {
            let Some(track_idx) = self.capturing_track.take() else {
                return;
            };
            // A free capture stopped instantly yields an empty buffer — skip.
            if samples.is_empty() || track_idx >= self.session.tracks.len() {
                return;
            }
            let media = self.store.put(&samples, self.sample_rate);
            let phrase = Phrase::new(
                media,
                self.session.bars_per_phrase,
                self.session.bpm,
                Self::now_ms(),
            );
            let layer = Layer::new(phrase.id);
            let track_id = self.session.tracks[track_idx].id;
            let layer_index = self.session.tracks[track_idx].layers.len() as u16;
            self.history.commit(
                Edit::AppendLayer {
                    track_id,
                    phrase,
                    layer,
                },
                &mut self.session,
                Self::now_ms(),
            );
            if self.track_is_audible(track_idx) {
                let key = LayerKey::new(track_id, layer_index);
                let clocked = self.session.master_clock_enabled;
                if let Ok(engine) = &mut self.engine {
                    let _ = if clocked {
                        engine.play_layer_at_next_bar(key, samples, 1.0, true)
                    } else {
                        engine.play_layer(key, samples, 1.0, true)
                    };
                }
                self.playing.push(key);
            }
        }
    }

    // === gestures (commit a real Edit, then drive the engine) ===

    /// Arm track `idx` (single-arm: unarm the previous holder). Stops a live
    /// capture first so the flag never points at an un-armed track.
    pub fn arm(&mut self, idx: usize) {
        if self.session.tracks.get(idx).is_none_or(|t| t.armed) {
            return;
        }
        if self.is_recording() {
            self.stop_capture();
        }
        let now = Self::now_ms();
        if let Some(prev) = self.armed_index() {
            let track_id = self.session.tracks[prev].id;
            self.history.commit(
                Edit::ArmTrack {
                    track_id,
                    from: true,
                    to: false,
                },
                &mut self.session,
                now,
            );
        }
        let track_id = self.session.tracks[idx].id;
        self.history.commit(
            Edit::ArmTrack {
                track_id,
                from: false,
                to: true,
            },
            &mut self.session,
            now,
        );
    }

    /// Add an empty track using the session's selected playback profile.
    pub fn add_track(&mut self) {
        let index = self.session.tracks.len();
        let track = Track::new_with_mode(
            format!("track {}", index + 1),
            TrackColor::from_palette_index(index),
            self.session.default_playback_mode,
        );
        self.history
            .commit(Edit::AddTrack { track }, &mut self.session, Self::now_ms());
    }

    /// The Record gesture. Master clock on → a bar-aligned, count-in, fixed
    /// capture. Master clock off → toggle a free/variable-length capture.
    pub fn toggle_record(&mut self) {
        let Some(armed) = self.armed_index() else {
            return;
        };
        if self.session.master_clock_enabled {
            let count_in = self.session.count_in_bars;
            if let Ok(engine) = &mut self.engine {
                if engine
                    .arm_bar_aligned_capture(self.session.bars_per_phrase, count_in)
                    .is_ok()
                {
                    self.capturing_track = Some(armed);
                }
            }
        } else if matches!(self.capture_phase, CapturePhase::FreeRecording { .. }) {
            self.stop_capture();
        } else if let Ok(engine) = &mut self.engine {
            if engine.start_free_capture().is_ok() {
                self.capturing_track = Some(armed);
            }
        }
    }

    /// Stop an in-flight free capture (the tick picks up the Complete buffer).
    fn stop_capture(&mut self) {
        if let Ok(engine) = &mut self.engine {
            engine.stop_free_capture();
        }
    }

    /// Toggle track-level mute (the lane's M): stop / replay the track's voices.
    pub fn toggle_track_mute(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else {
            return;
        };
        let (track_id, from) = (track.id, track.muted);
        self.history.commit(
            Edit::MuteTrack {
                track_id,
                from,
                to: !from,
            },
            &mut self.session,
            Self::now_ms(),
        );
        if !from {
            // Now muted: stop every voice on this track.
            let keys: Vec<LayerKey> = self
                .playing
                .iter()
                .copied()
                .filter(|k| k.track_id == track_id)
                .collect();
            for key in keys {
                self.stop_layer_key(key);
            }
        } else if self.track_is_audible(idx) {
            self.reconcile_track_playback(idx);
        }
    }

    /// Toggle one layer's mute (tap a layer row): stop / replay that voice.
    pub fn toggle_layer_mute(&mut self, track_idx: usize, layer_index: u16) {
        let Some(track) = self.session.tracks.get(track_idx) else {
            return;
        };
        let track_id = track.id;
        let Some(layer) = track.layers.get(layer_index as usize) else {
            return;
        };
        let from = layer.muted;
        self.history.commit(
            Edit::SetLayerMute {
                track_id,
                layer_index,
                from,
                to: !from,
            },
            &mut self.session,
            Self::now_ms(),
        );
        let key = LayerKey::new(track_id, layer_index);
        if !from {
            self.stop_layer_key(key);
        } else if self.track_is_audible(track_idx) {
            self.play_layer_from_store(track_idx, layer_index as usize);
        }
    }

    /// Nudge tempo (clamped) and re-sync the engine grid.
    pub fn bpm_nudge(&mut self, delta: f32) {
        let from = self.session.bpm;
        let to = (from + delta).clamp(40.0, 240.0);
        if (to - from).abs() < f32::EPSILON {
            return;
        }
        self.history
            .commit(Edit::SetBpm { from, to }, &mut self.session, Self::now_ms());
        self.resync_tempo();
    }

    /// Toggle the master clock (bar-aligned capture + count-in). Model-only;
    /// the audible metronome is the separate [`toggle_click`](Self::toggle_click).
    pub fn toggle_master_clock(&mut self) {
        let from = self.session.master_clock_enabled;
        self.history.commit(
            Edit::SetMasterClock { from, to: !from },
            &mut self.session,
            Self::now_ms(),
        );
    }

    /// Toggle the audible metronome click on the engine.
    pub fn toggle_click(&mut self) {
        self.click = !self.click;
        if let Ok(engine) = &mut self.engine {
            engine.set_click_enabled(self.click);
        }
    }

    /// Toggle solo membership for track `idx` and reconcile the engine with the
    /// resulting audible-track set.
    pub fn toggle_solo(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else {
            return;
        };
        let id = track.id;
        if !self.solo.remove(&id) {
            self.solo.insert(id);
        }
        self.reconcile_all_playback();
    }

    // === engine playback helpers ===

    /// Push tempo + full time signature to the click and re-apply the click enable.
    /// Stops all playing loops first — they were captured at the old grid and
    /// would drift.
    fn resync_tempo(&mut self) {
        self.stop_all();
        let bpm = self.session.bpm;
        let time_signature = self.session.time_signature;
        let click = self.click;
        if let Ok(engine) = &mut self.engine {
            let _ = engine.set_tempo(bpm, time_signature.numerator, time_signature.denominator);
            engine.set_click_enabled(click);
        }
    }

    fn apply_audio_device_selection(&mut self) {
        let input = self.audio_input_select.selected;
        let output = self.audio_output_select.selected;
        if input == self.applied_audio_input && output == self.applied_audio_output {
            return;
        }
        if self.is_recording() {
            self.audio_input_select.selected = self.applied_audio_input;
            self.audio_output_select.selected = self.applied_audio_output;
            self.audio_status = "stop capture before changing audio".to_string();
            return;
        }

        let selection = AudioDeviceSelection {
            input_id: selected_device_id(&self.audio_devices.inputs, input),
            output_id: selected_device_id(&self.audio_devices.outputs, output),
        };
        self.stop_all();
        let old_engine = std::mem::replace(&mut self.engine, Err("restarting audio".to_string()));
        drop(old_engine);

        match Engine::new_with_audio_devices(&selection) {
            Ok(engine) => {
                self.sample_rate = engine.sample_rate();
                self.engine = Ok(engine);
                self.applied_audio_input = input;
                self.applied_audio_output = output;
                self.audio_status = "audio switched".to_string();
                self.resync_tempo();
                self.reconcile_all_playback();
            }
            Err(error) => {
                self.audio_input_select.selected = self.applied_audio_input;
                self.audio_output_select.selected = self.applied_audio_output;
                match Engine::new() {
                    Ok(engine) => {
                        self.sample_rate = engine.sample_rate();
                        self.engine = Ok(engine);
                        self.audio_input_select.selected = 0;
                        self.audio_output_select.selected = 0;
                        self.applied_audio_input = 0;
                        self.applied_audio_output = 0;
                        self.audio_status =
                            format!("audio switch failed: {error}; restored default");
                        self.resync_tempo();
                        self.reconcile_all_playback();
                    }
                    Err(fallback_error) => {
                        self.engine = Err(format!(
                            "{error}; default audio also failed: {fallback_error}"
                        ));
                        self.audio_status = "audio unavailable".to_string();
                    }
                }
            }
        }
    }

    /// Stop every live loop. The session layers remain intact and can be
    /// projected back into the engine after a transport command or state change.
    pub fn stop_all(&mut self) {
        let keys: Vec<LayerKey> = self.playing.drain(..).collect();
        if let Ok(engine) = &mut self.engine {
            for key in keys {
                engine.stop_layer(key);
            }
        }
    }

    fn stop_layer_key(&mut self, key: LayerKey) {
        if let Ok(engine) = &mut self.engine {
            engine.stop_layer(key);
        }
        self.playing.retain(|k| *k != key);
    }

    /// (Re)play every audible layer of track `idx` from the store.
    fn reconcile_track_playback(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else {
            return;
        };
        let audible: Vec<usize> = track
            .layers
            .iter()
            .enumerate()
            .filter(|(li, layer)| {
                track
                    .playback_mode
                    .is_layer_audible(*li as u16, layer.muted)
            })
            .map(|(li, _)| li)
            .collect();
        for li in audible {
            self.play_layer_from_store(idx, li);
        }
    }

    fn track_is_audible(&self, idx: usize) -> bool {
        let Some(track) = self.session.tracks.get(idx) else {
            return false;
        };
        !track.muted && (self.solo.is_empty() || self.solo.contains(&track.id))
    }

    fn reconcile_all_playback(&mut self) {
        self.stop_all();
        let audible: Vec<usize> = (0..self.session.tracks.len())
            .filter(|&idx| self.track_is_audible(idx))
            .collect();
        for idx in audible {
            self.reconcile_track_playback(idx);
        }
    }

    /// Play a layer from the store at the next bar, at its stored gain. No-op
    /// when the media is unavailable in the current store.
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
        let sample_rate = buf.sample_rate;
        let (gain, key) = (layer.gain, LayerKey::new(track.id, layer_idx as u16));
        if let Ok(engine) = &mut self.engine {
            let _ =
                engine.play_layer_at_next_bar_at_sample_rate(key, samples, sample_rate, gain, true);
        }
        if !self.playing.contains(&key) {
            self.playing.push(key);
        }
    }
}

fn ensure_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension(extension);
    }
    path
}

fn audio_device_options(devices: &[strophe_engine::AudioDevice]) -> Vec<String> {
    std::iter::once("System default".to_string())
        .chain(devices.iter().map(|device| {
            if device.is_default {
                format!("{} (default)", device.name)
            } else {
                device.name.clone()
            }
        }))
        .collect()
}

fn selected_device_id(devices: &[strophe_engine::AudioDevice], selected: usize) -> Option<String> {
    devices
        .get(selected.checked_sub(1)?)
        .map(|device| device.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_options_reserve_zero_for_the_system_default() {
        let devices = vec![strophe_engine::AudioDevice {
            id: "input-1".to_string(),
            name: "Studio input".to_string(),
            is_default: true,
        }];
        assert_eq!(
            audio_device_options(&devices),
            vec!["System default", "Studio input (default)"]
        );
        assert_eq!(selected_device_id(&devices, 0), None);
        assert_eq!(selected_device_id(&devices, 1).as_deref(), Some("input-1"));
    }

    #[test]
    fn meter_db_normalization_has_an_explicit_floor() {
        assert_eq!(normalize_meter_db(f32::NEG_INFINITY), 0.0);
        assert_eq!(normalize_meter_db(-60.0), 0.0);
        assert!((normalize_meter_db(-30.0) - 0.5).abs() < f32::EPSILON);
        assert_eq!(normalize_meter_db(0.0), 1.0);
    }
}

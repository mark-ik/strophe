//! S5: chisel leaves for the loop waveforms + the output meter.
//!
//! The signature visual — each track's summed loop — is now a chisel Path-A
//! leaf (a filled, mirrored amplitude envelope) rather than a row of CSS bars.
//! The output meter is chisel's built-in [`chisel::Meter`]. The view places a
//! `<chisel-leaf key=…>` box; the host owns the leaves out of band in a
//! [`chisel::LeafRegistry`] and reconciles them from [`AppState`] each frame,
//! so serval stays a uniform-DOM engine and the widget content lives host-side.
//!
//! Keys are derived from track index (waveforms) + a small fixed namespace
//! (meters), so the view and the host agree without threading ids through the
//! `<chisel-leaf>` element (which carries only key + box).

use chisel::{ColorF, Leaf, LeafRegistry, PaintCx, Path, RenderedLeaves, Size, SizeHint};
use paint_list_api::PaintCmd;
use serval_layout::LeafPaintSource;
use strophe_model::TrackColor;

use crate::state::AppState;

// --- key scheme ---------------------------------------------------------

/// Summed-waveform leaf key for track `i`. One namespace band per leaf family
/// keeps the u64 space collision-free and readable in logs.
pub fn wave_key(track: usize) -> u64 {
    0x5741_0000 + track as u64 // "WA"
}
/// The two output-meter leaves (L / R).
pub const METER_L: u64 = 0x4D45_0000; // "ME"
pub const METER_R: u64 = 0x4D45_0001;

// --- the waveform leaf --------------------------------------------------

/// A filled, mirrored amplitude envelope. Peaks are `0..1` column samples; the
/// leaf paints a closed polygon from the top envelope across and back along the
/// mirrored bottom, filled with the owner's colour. Resolution-independent and
/// tile-cached — the payoff over the CSS-bar stand-in.
pub struct WaveformLeaf {
    peaks: Vec<f32>,
    color: ColorF,
    intrinsic: Size,
    /// Content signature (owner colour + peak fingerprint). Repaint only when it
    /// moves — a stable take paints once, then the retention gate holds.
    sig: u64,
    dirty: bool,
}

impl WaveformLeaf {
    fn new(peaks: Vec<f32>, color: ColorF, sig: u64) -> Self {
        Self {
            peaks,
            color,
            intrinsic: Size { width: 260.0, height: 40.0 },
            sig,
            dirty: true,
        }
    }

    /// Re-seed from new content only when the signature moved — the peak Vec is
    /// computed lazily so an unchanged take costs nothing on the redraw path.
    fn update(&mut self, peaks: impl FnOnce() -> Vec<f32>, color: ColorF, sig: u64) {
        if sig != self.sig {
            self.peaks = peaks();
            self.color = color;
            self.sig = sig;
            self.dirty = true;
        }
    }
}

impl Leaf for WaveformLeaf {
    fn measure(&mut self, _known: SizeHint, _available: SizeHint) -> Size {
        self.intrinsic
    }

    fn paint(&mut self, cx: &mut PaintCx<'_>) {
        let s = cx.size();
        let n = self.peaks.len().max(1);
        let mid = s.height * 0.5;
        // A minimum half-height so a near-silent column still reads as a line.
        let half = |p: f32| (p * (s.height * 0.5 - 1.0)).max(1.0);
        let x_at = |i: usize| (i as f32 / (n - 1).max(1) as f32) * s.width;

        // Top envelope left→right, then the mirrored bottom right→left, closed.
        let mut path = Path::new().move_to(0.0, mid - half(self.peaks[0]));
        for i in 1..n {
            path = path.line_to(x_at(i), mid - half(self.peaks[i]));
        }
        for i in (0..n).rev() {
            path = path.line_to(x_at(i), mid + half(self.peaks[i]));
        }
        cx.fill_path(path.close().build(), self.color);
        self.dirty = false;
    }

    fn paint_dirty(&self) -> bool {
        self.dirty
    }
}

// --- reconcile from AppState -------------------------------------------

fn color_of(c: TrackColor) -> ColorF {
    ColorF {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: 1.0,
    }
}

/// Deterministic envelope for a track's audible sum — a musical-phrase shape
/// times a stable pseudo-random, seeded from the layer phrase ids so a take
/// keeps its silhouette. Mirrors `view.rs`'s stand-in until real peak data
/// arrives from the engine.
fn summed_peaks(seed: u64, n: usize) -> Vec<f32> {
    let mut s = seed ^ 0x9e37_79b9_7f4a_7c15;
    (0..n)
        .map(|i| {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let r = ((s >> 33) & 0x7fff) as f32 / 32_767.0;
            let t = i as f32 / n as f32;
            let env = 0.35 + 0.65 * (std::f32::consts::PI * t).sin().powf(0.6);
            (0.25 + r * 0.75) * env
        })
        .collect()
}

/// A signature of a track's audible content: its unmuted layers' phrase ids.
/// Moves when a layer is added, muted, or unmuted — which is exactly when the
/// summed envelope should re-seed.
fn track_sig(track: &strophe_model::Track) -> u64 {
    track
        .layers
        .iter()
        .filter(|l| !l.muted)
        .fold(0xcbf2_9ce4_8422_2325u64, |acc, l| {
            let b = l.phrase_id.0.as_bytes();
            let word = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            (acc ^ word).wrapping_mul(0x0100_0000_01b3)
        })
}

/// Ensure the registry holds exactly the leaves the current session needs, with
/// up-to-date content. Called each frame before rendering; cheap when nothing
/// changed (signatures match → no re-seed, no repaint).
pub fn reconcile(registry: &mut LeafRegistry<u64>, state: &AppState) {
    // Waveform per non-empty track.
    for (i, track) in state.session.tracks.iter().enumerate() {
        let key = wave_key(i);
        if track.layers.iter().all(|l| l.muted) || track.layers.is_empty() {
            registry.remove(&key);
            continue;
        }
        // Fold colour into the signature so a recolour repaints too.
        let c = track.color;
        let sig = track_sig(track) ^ ((c.r as u64) << 16 | (c.g as u64) << 8 | c.b as u64);
        let color = color_of(c);
        if let Some(leaf) = registry.get_mut_as::<WaveformLeaf>(&key) {
            leaf.update(|| summed_peaks(sig, 96), color, sig);
        } else {
            registry.insert(key, Box::new(WaveformLeaf::new(summed_peaks(sig, 96), color, sig)));
        }
    }
    // Output meters (placeholder levels until the engine feeds real ones).
    ensure_meter(registry, METER_L, 0.72);
    ensure_meter(registry, METER_R, 0.61);
}

fn ensure_meter(registry: &mut LeafRegistry<u64>, key: u64, level: f32) {
    if let Some(m) = registry.get_mut_as::<chisel::Meter>(&key) {
        m.set_level(level, Some(level));
    } else {
        let mut m = chisel::Meter::new(true, Size { width: 10.0, height: 46.0 });
        // Match the sheet: teal fill on a dim track, amber peak tick.
        m.track_color = ColorF { r: 0.16, g: 0.14, b: 0.10, a: 1.0 };
        m.fill_color = ColorF { r: 0.34, g: 0.70, b: 0.66, a: 1.0 };
        m.peak_color = ColorF { r: 0.88, g: 0.65, b: 0.29, a: 1.0 };
        m.set_level(level, Some(level));
        registry.insert(key, Box::new(m));
    }
}

// --- LeafPaintSource adapter -------------------------------------------

/// Forwards serval-layout's per-leaf command query to chisel's rendered cache.
/// A newtype because both traits live in other crates (orphan rule).
pub struct LeafSource<'a>(pub &'a RenderedLeaves);

impl LeafPaintSource for LeafSource<'_> {
    fn leaf_commands(&self, key: u64) -> Option<&[PaintCmd]> {
        self.0.get(key)
    }
}

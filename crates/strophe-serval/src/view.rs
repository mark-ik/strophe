//! The Strophe UI as serval views (2026-07-08 concept; S2: real data).
//!
//! One screen: the pass-the-mic circle | the loop table | the transport.
//! Everything data-bearing derives from [`AppState`]'s `Session`; gestures
//! call the `AppState` methods, which commit real `Edit`s. The rail's peers
//! are placeholders until the sync layer. The waveform + meter are DOM
//! stand-ins; S5 makes them chisel leaves.

use strophe_model::{PlaybackMode, Track, TrackColor};
use xilem_serval::{clickable, el, text, AnyView, ServalCtx, ServalElement};

use crate::leaves::{wave_key, METER_L, METER_R};
use crate::state::{AppState, OWNERS};

pub type Child = Box<dyn AnyView<AppState, (), ServalCtx, ServalElement>>;

/// A `<chisel-leaf>` block: carries only its key + box; the host renders the
/// registered leaf's Path-A commands into it (see [`crate::leaves`]).
fn chisel_leaf(key: u64, w: u32, h: u32) -> Child {
    Box::new(
        el("chisel-leaf", ())
            .attr("key", key.to_string())
            .attr("style", format!("display: block; width: {w}px; height: {h}px")),
    )
}

/// The summed-loop waveform leaf for track `i`.
fn wave_leaf(i: usize) -> Child {
    chisel_leaf(wave_key(i), 280, 40)
}

fn hex(c: TrackColor) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Deterministic waveform heights in `0..1` — a musical-phrase envelope
/// times a stable pseudo-random, so the stand-in reads like audio. Seeded
/// per layer from its phrase id, so a layer keeps its shape across frames.
fn heights(seed: u32, n: usize) -> Vec<f32> {
    let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let r = ((s >> 16) & 0x7fff) as f32 / 32_767.0;
        let t = i as f32 / n as f32;
        let env = 0.35 + 0.65 * (std::f32::consts::PI * t).sin().powf(0.6);
        out.push((0.25 + r * 0.75) * env);
    }
    out
}

fn bars(seed: u32, n: usize) -> Vec<Child> {
    heights(seed, n)
        .iter()
        .map(|h| {
            Box::new(
                el("div", ())
                    .attr("class", "bar")
                    .attr("style", format!("height: {}%", (h * 100.0).max(6.0) as u32)),
            ) as Child
        })
        .collect()
}

/// A stable per-layer seed from the phrase id's leading bytes.
fn layer_seed(track: &Track, layer_index: usize) -> u32 {
    let b = track.layers[layer_index].phrase_id.0.as_bytes();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub fn root(state: &AppState) -> Child {
    Box::new(el("div", (top(state), body(state), transport(state))).attr("class", "app"))
}

fn top(state: &AppState) -> Child {
    let profile = match state.session.default_playback_mode {
        PlaybackMode::Sum => "looper-pedal",
        PlaybackMode::SelectOne { .. } => "deeler",
    };
    let chip = format!("{} tracks \u{00b7} {}", state.session.tracks.len(), profile);
    Box::new(
        el(
            "div",
            (
                el(
                    "span",
                    (
                        el("span", text("strophe")),
                        el("span", text(".")).attr("class", "brand-dot"),
                    ),
                )
                .attr("class", "brand"),
                el("span", text("\"back porch, take 3\"")).attr("class", "session-name mono"),
                el("div", ()).attr("class", "top-spacer"),
                el("span", text(chip)).attr("class", "chip mono"),
                el("div", text("\u{2699}")).attr("class", "cog"),
            ),
        )
        .attr("class", "top"),
    )
}

fn body(state: &AppState) -> Child {
    Box::new(el("div", (rail(state), table(state))).attr("class", "body"))
}

fn peer(name: &str, initials: &str, voice: &str, sub: &str, turn: bool) -> Child {
    let cls = if turn { "peer peer-turn" } else { "peer" };
    let av = el("div", text(initials.to_string()))
        .attr("class", "av")
        .attr("style", format!("background-color: {voice}"));
    let who = el(
        "div",
        (
            el("span", text(name.to_string())).attr("class", "who-name"),
            el("span", text(sub.to_string())).attr("class", "who-sub"),
        ),
    )
    .attr("class", "who");
    let mut kids: Vec<Child> = vec![Box::new(av), Box::new(who)];
    if turn {
        kids.push(Box::new(
            el("span", text("\u{25cf}"))
                .attr("class", "mic")
                .attr("style", format!("color: {voice}")),
        ));
    }
    Box::new(el("div", kids).attr("class", cls))
}

/// The pass-the-mic rail. Peers are placeholder data until the sync layer;
/// "You" reflects the live arm/record state.
fn rail(state: &AppState) -> Child {
    let you_sub = if state.recording {
        "your turn \u{00b7} recording"
    } else {
        "your turn"
    };
    let amber = "#e0a64b";
    let kids: Vec<Child> = vec![
        Box::new(el("div", text("the circle")).attr("class", "eyebrow")),
        peer("You", "YU", amber, you_sub, true),
        peer("Jonah", "JD", "#56b3a8", "laid down bass", false),
        peer("Mara", "MR", "#e0796a", "waiting", false),
        peer("Eli", "EL", "#a9b96b", "waiting", false),
        Box::new(el("div", ()).attr("class", "rail-spacer")),
        Box::new(clickable(
            el("div", text("Hand off \u{2192}")).attr("class", "handoff"),
            |_state: &mut AppState, _| {},
        )),
        Box::new(el("div", text("passes the mic to Jonah")).attr("class", "handoff-note")),
    ];
    Box::new(el("div", kids).attr("class", "rail"))
}

fn table(state: &AppState) -> Child {
    let mut kids: Vec<Child> = vec![Box::new(
        el(
            "div",
            (
                el("div", text("loops")).attr("class", "eyebrow"),
                el("div", text("tap a dot to arm \u{00b7} tap a layer to mute"))
                    .attr("class", "table-hint"),
            ),
        )
        .attr("class", "table-head"),
    )];
    for i in 0..state.session.tracks.len() {
        kids.push(lane(state, i));
    }
    kids.push(Box::new(clickable(
        el("div", text("+ add track")).attr("class", "add-track"),
        |_state: &mut AppState, _| {},
    )));
    Box::new(el("div", kids).attr("class", "table"))
}

fn lane(state: &AppState, i: usize) -> Child {
    let track = &state.session.tracks[i];
    let recording = track.armed && state.recording;
    let mut cls = String::from("lane");
    if recording {
        cls.push_str(" lane-rec");
    } else if track.armed {
        cls.push_str(" lane-armed");
    }
    let style = format!("--voice: {}", hex(track.color));
    let owner = OWNERS.get(i).copied().unwrap_or("you");
    let n = track.layers.len();

    let id = el(
        "div",
        (
            el(
                "div",
                (
                    clickable(el("div", ()).attr("class", "arm"), move |state: &mut AppState, _| {
                        state.arm(i);
                    }),
                    el("span", text(track.name.clone())).attr("class", "lane-title"),
                ),
            )
            .attr("class", "lane-name"),
            el(
                "div",
                (
                    el("span", text(owner.to_string())).attr("class", "tag tag-voice"),
                    el(
                        "span",
                        text(if n == 0 {
                            "empty".to_string()
                        } else {
                            format!("{n} layers")
                        }),
                    )
                    .attr("class", "tag mono"),
                ),
            )
            .attr("class", "lane-meta"),
        ),
    )
    .attr("class", "lane-id");

    let wavecol: Child = if n == 0 {
        Box::new(
            el(
                "div",
                el("span", text("no layers yet \u{00b7} arm and record to start the loop"))
                    .attr("class", "wave-empty"),
            )
            .attr("class", "lane-wave"),
        )
    } else {
        // Newest layer on top: display index li walks the stack from the end.
        let layer_rows: Vec<Child> = (0..n)
            .rev()
            .map(|li| {
                let muted = track.layers[li].muted;
                let cls = if muted { "layer layer-muted" } else { "layer" };
                Box::new(clickable(
                    el(
                        "div",
                        (
                            el("span", text(format!("L{}", li + 1))).attr("class", "lnum mono"),
                            el("div", bars(layer_seed(track, li), 26)).attr("class", "layer-wave"),
                        ),
                    )
                    .attr("class", cls),
                    move |state: &mut AppState, _| {
                        state.toggle_layer_mute(i, li as u16);
                    },
                )) as Child
            })
            .collect();
        Box::new(
            el(
                "div",
                (
                    // The summed loop is a chisel Path-A leaf (host-owned, keyed
                    // by track); its filled envelope re-seeds when the audible
                    // stack changes. See `leaves::reconcile`.
                    wave_leaf(i),
                    el("div", layer_rows).attr("class", "layers"),
                ),
            )
            .attr("class", "lane-wave"),
        )
    };

    let m_cls = if track.muted { "lctl lctl-on" } else { "lctl" };
    let s_cls = if state.solo.contains(&track.id) { "lctl lctl-on" } else { "lctl" };
    let ctl = el(
        "div",
        (
            clickable(
                el("div", text("M")).attr("class", m_cls),
                move |state: &mut AppState, _| state.toggle_track_mute(i),
            ),
            clickable(
                el("div", text("S")).attr("class", s_cls),
                move |state: &mut AppState, _| state.toggle_solo(i),
            ),
            el("div", text("\u{21bb}")).attr("class", "lctl"),
        ),
    )
    .attr("class", "lane-ctl");

    Box::new(
        el("div", (id, wavecol, ctl))
            .attr("class", cls)
            .attr("style", style),
    )
}

fn transport(state: &AppState) -> Child {
    let rec_cls = if state.recording { "record record-armed" } else { "record" };
    let bpm = format!("{}", state.session.bpm.round() as u32);
    let ts = state.session.time_signature;
    let meter_txt = format!("{}/{}", ts.numerator, ts.denominator);
    let rec_label = match state.armed_index() {
        Some(i) => format!(" \u{00b7} {} armed", state.session.tracks[i].name),
        None => " \u{00b7} nothing armed".to_string(),
    };
    let click_cls = if state.click { "toggle toggle-on" } else { "toggle" };
    let clock_cls = if state.session.master_clock_enabled {
        "toggle toggle-on"
    } else {
        "toggle"
    };

    let left = el(
        "div",
        (
            el(
                "div",
                (
                    el("div", text("tempo")).attr("class", "eyebrow"),
                    el(
                        "div",
                        (
                            clickable(
                                el("div", text("\u{2212}")).attr("class", "step"),
                                |state: &mut AppState, _| state.bpm_nudge(-2.0),
                            ),
                            el("span", text(bpm)).attr("class", "step-val mono"),
                            clickable(
                                el("div", text("+")).attr("class", "step"),
                                |state: &mut AppState, _| state.bpm_nudge(2.0),
                            ),
                        ),
                    )
                    .attr("class", "stepper"),
                ),
            )
            .attr("class", "readout"),
            el(
                "div",
                (
                    el("div", text("meter")).attr("class", "eyebrow"),
                    el("div", text(meter_txt)).attr("class", "readout-val mono"),
                ),
            )
            .attr("class", "readout"),
            el(
                "div",
                (
                    clickable(
                        el(
                            "div",
                            (el("span", ()).attr("class", "led"), el("span", text("Click"))),
                        )
                        .attr("class", click_cls),
                        |state: &mut AppState, _| state.toggle_click(),
                    ),
                    clickable(
                        el(
                            "div",
                            (
                                el("span", ()).attr("class", "led"),
                                el("span", text("Master clock")),
                            ),
                        )
                        .attr("class", clock_cls),
                        |state: &mut AppState, _| state.toggle_master_clock(),
                    ),
                ),
            )
            .attr("class", "toggles"),
        ),
    )
    .attr("class", "t-left");

    let center = el(
        "div",
        (
            clickable(
                el("div", el("div", ()).attr("class", "record-core")).attr("class", rec_cls),
                |state: &mut AppState, _| state.toggle_record(),
            ),
            el(
                "div",
                (el("b", text("Record")), el("span", text(rec_label))),
            )
            .attr("class", "record-label"),
        ),
    )
    .attr("class", "record-wrap");

    let right = el(
        "div",
        el(
            "div",
            (el("div", text("\u{25a0}")).attr("class", "stop"), meter()),
        )
        .attr("class", "t-right-inner"),
    )
    .attr("class", "t-right");

    Box::new(el("div", (left, center, right)).attr("class", "transport"))
}

fn meter() -> Child {
    // The L/R output bars are chisel `Meter` leaves (host-owned).
    let col = |key: u64, label: &str| -> Child {
        Box::new(
            el(
                "div",
                (
                    chisel_leaf(key, 10, 46),
                    el("span", text(label.to_string())).attr("class", "mlbl mono"),
                ),
            )
            .attr("class", "mcol"),
        )
    };
    Box::new(
        el(
            "div",
            (
                el("div", (col(METER_L, "L"), col(METER_R, "R"))).attr("class", "mbars"),
                el(
                    "div",
                    (
                        el("div", text("out")).attr("class", "eyebrow"),
                        el("div", text("\u{2212}4.2 dB")).attr("class", "mpeak mono"),
                    ),
                )
                .attr("class", "readout"),
            ),
        )
        .attr("class", "meter"),
    )
}

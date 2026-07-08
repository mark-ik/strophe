//! The Strophe UI as serval views (2026-07-08 concept, S1: static shape).
//!
//! One screen: the pass-the-mic circle | the loop table | the transport.
//! Data is placeholder this slice; S2 wires it to `AppState`. The waveform
//! + meter are lightweight DOM stand-ins; S5 makes them chisel leaves. One
//! interaction is live (arm a track / toggle record) to prove the loop.

use xilem_serval::{clickable, el, text, AnyView, ServalCtx, ServalElement};

pub type Child = Box<dyn AnyView<Ui, (), ServalCtx, ServalElement>>;

/// S1 placeholder state.
pub struct Ui {
    /// Which track holds the arm (0..4).
    pub armed: usize,
    /// Whether the armed track is capturing (the record light).
    pub recording: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self { armed: 0, recording: true }
    }
}

fn voice_hex(v: &str) -> &'static str {
    match v {
        "teal" => "#56b3a8",
        "coral" => "#e0796a",
        "sage" => "#a9b96b",
        _ => "#e0a64b",
    }
}

/// Deterministic waveform heights in `0..1` — a musical-phrase envelope
/// times a stable pseudo-random, so the stand-in reads like audio.
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

fn seed_for(track: usize, layer: usize) -> u32 {
    (track as u32 + 1) * 71 + layer as u32 * 13 + 3
}

pub fn root(ui: &Ui) -> Child {
    Box::new(el("div", (top(), body(ui), transport(ui))).attr("class", "app"))
}

fn top() -> Child {
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
                el("span", text("4 tracks \u{00b7} looper-pedal")).attr("class", "chip mono"),
                el("div", text("\u{2699}")).attr("class", "cog"),
            ),
        )
        .attr("class", "top"),
    )
}

fn body(ui: &Ui) -> Child {
    Box::new(el("div", (rail(), table(ui))).attr("class", "body"))
}

fn peer(name: &str, initials: &str, voice: &str, sub: &str, turn: bool) -> Child {
    let cls = if turn { "peer peer-turn" } else { "peer" };
    let hex = voice_hex(voice);
    let av = el("div", text(initials.to_string()))
        .attr("class", "av")
        .attr("style", format!("background-color: {hex}"));
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
                .attr("style", format!("color: {hex}")),
        ));
    }
    Box::new(el("div", kids).attr("class", cls))
}

fn rail() -> Child {
    let kids: Vec<Child> = vec![
        Box::new(el("div", text("the circle")).attr("class", "eyebrow")),
        peer("You", "YU", "amber", "your turn \u{00b7} recording", true),
        peer("Jonah", "JD", "teal", "laid down bass", false),
        peer("Mara", "MR", "coral", "waiting", false),
        peer("Eli", "EL", "sage", "waiting", false),
        Box::new(el("div", ()).attr("class", "rail-spacer")),
        Box::new(clickable(
            el("div", text("Hand off \u{2192}")).attr("class", "handoff"),
            |_ui: &mut Ui, _| {},
        )),
        Box::new(el("div", text("passes the mic to Jonah")).attr("class", "handoff-note")),
    ];
    Box::new(el("div", kids).attr("class", "rail"))
}

fn table(ui: &Ui) -> Child {
    let kids: Vec<Child> = vec![
        Box::new(
            el(
                "div",
                (
                    el("div", text("loops")).attr("class", "eyebrow"),
                    el("div", text("tap a dot to arm \u{00b7} tap a layer to mute"))
                        .attr("class", "table-hint"),
                ),
            )
            .attr("class", "table-head"),
        ),
        lane(ui, 0, "Guitar", "you", "amber", &[false, false, true], false, false),
        lane(ui, 1, "Bass", "jonah", "teal", &[false, false], false, false),
        lane(ui, 2, "Drums", "mara", "coral", &[false, false, true, false], false, true),
        lane(ui, 3, "Keys", "eli", "sage", &[], true, false),
        Box::new(clickable(
            el("div", text("+ add track")).attr("class", "add-track"),
            |_ui: &mut Ui, _| {},
        )),
    ];
    Box::new(el("div", kids).attr("class", "table"))
}

#[allow(clippy::too_many_arguments)]
fn lane(
    ui: &Ui,
    i: usize,
    name: &str,
    owner: &str,
    voice: &str,
    layers: &[bool],
    empty: bool,
    solo: bool,
) -> Child {
    let armed = ui.armed == i;
    let recording = armed && ui.recording;
    let mut cls = String::from("lane");
    if recording {
        cls.push_str(" lane-rec");
    } else if armed {
        cls.push_str(" lane-armed");
    }
    let style = format!("--voice: {}", voice_hex(voice));

    let id = el(
        "div",
        (
            el(
                "div",
                (
                    clickable(el("div", ()).attr("class", "arm"), move |ui: &mut Ui, _| {
                        ui.armed = i;
                    }),
                    el("span", text(name.to_string())).attr("class", "lane-title"),
                ),
            )
            .attr("class", "lane-name"),
            el(
                "div",
                (
                    el("span", text(owner.to_string())).attr("class", "tag tag-voice"),
                    el(
                        "span",
                        text(if empty {
                            "empty".to_string()
                        } else {
                            format!("{} layers", layers.len())
                        }),
                    )
                    .attr("class", "tag mono"),
                ),
            )
            .attr("class", "lane-meta"),
        ),
    )
    .attr("class", "lane-id");

    let wavecol: Child = if empty {
        Box::new(
            el(
                "div",
                el("span", text("no layers yet \u{00b7} arm and record to start the loop"))
                    .attr("class", "wave-empty"),
            )
            .attr("class", "lane-wave"),
        )
    } else {
        let n = layers.len();
        let layer_rows: Vec<Child> = layers
            .iter()
            .enumerate()
            .map(|(li, muted)| {
                let lnum = n - li; // L3, L2, L1 top-to-bottom
                let cls = if *muted { "layer layer-muted" } else { "layer" };
                Box::new(
                    el(
                        "div",
                        (
                            el("span", text(format!("L{lnum}"))).attr("class", "lnum mono"),
                            el("div", bars(seed_for(i, lnum), 26)).attr("class", "layer-wave"),
                        ),
                    )
                    .attr("class", cls),
                ) as Child
            })
            .collect();
        Box::new(
            el(
                "div",
                (
                    el("div", bars(seed_for(i, 0), 30)).attr("class", "wave-summed"),
                    el("div", layer_rows).attr("class", "layers"),
                ),
            )
            .attr("class", "lane-wave"),
        )
    };

    let ctl = el(
        "div",
        (
            el("div", text("M")).attr("class", "lctl"),
            el("div", text("S")).attr("class", if solo { "lctl lctl-on" } else { "lctl" }),
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

fn transport(ui: &Ui) -> Child {
    let rec_cls = if ui.recording { "record record-armed" } else { "record" };
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
                            el("div", text("\u{2212}")).attr("class", "step"),
                            el("span", text("92")).attr("class", "step-val mono"),
                            el("div", text("+")).attr("class", "step"),
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
                    el("div", text("4/4")).attr("class", "readout-val mono"),
                ),
            )
            .attr("class", "readout"),
            el(
                "div",
                (
                    el(
                        "div",
                        (el("span", ()).attr("class", "led"), el("span", text("Click"))),
                    )
                    .attr("class", "toggle toggle-on"),
                    el(
                        "div",
                        (
                            el("span", ()).attr("class", "led"),
                            el("span", text("Master clock")),
                        ),
                    )
                    .attr("class", "toggle toggle-on"),
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
                |ui: &mut Ui, _| ui.recording = !ui.recording,
            ),
            el(
                "div",
                (el("b", text("Record")), el("span", text(" \u{00b7} Guitar armed"))),
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
    Box::new(
        el(
            "div",
            (
                el("div", (meter_col(0.72, "L"), meter_col(0.61, "R"))).attr("class", "mbars"),
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

fn meter_col(level: f32, label: &str) -> Child {
    let total = 12usize;
    let lit = (total as f32 * level).round() as usize;
    let segs: Vec<Child> = (0..total)
        .map(|i| {
            let frac = i as f32 / total as f32;
            let color = if frac > 0.86 {
                "var(--record)"
            } else if frac > 0.66 {
                "var(--voice-amber)"
            } else {
                "var(--voice-teal)"
            };
            let opacity = if i < lit { "1" } else { "0.14" };
            Box::new(
                el("div", ()).attr("class", "mseg").attr(
                    "style",
                    format!("width: 12px; background-color: {color}; opacity: {opacity}"),
                ),
            ) as Child
        })
        .collect();
    Box::new(
        el(
            "div",
            (
                el("div", segs).attr("class", "mbar-stack"),
                el("span", text(label.to_string())).attr("class", "mlbl mono"),
            ),
        )
        .attr("class", "mcol"),
    )
}

//! Transport bar — surface navigation, record/stop, capture phase, and
//! the output meter. The one control surface that stays put across all
//! other surfaces, per the "hand-on-instrument" north star.

use masonry::layout::AsUnit;
use xilem::style::Style;
use xilem::view::{flex_col, flex_row, label, sized_box, text_button, AnyFlexChild, FlexExt};
use xilem::WidgetView;

use strophe_engine::CapturePhase;
use strophe_widgets::theme::{mono_family, SP_1, SP_2, SP_3, TS_SM, TS_XL};
use strophe_widgets::{db_to_norm, meter_view};

use crate::view::Surface;
use crate::{capture_phase_text, fmt_db, AppState, METER_FLOOR_DB, METER_H, METER_W};

pub fn transport(state: &AppState) -> impl WidgetView<AppState> + use<> {
    let status = match &state.engine {
        Ok(_) => format!("engine @ {} Hz", state.sample_rate),
        Err(e) => format!("engine unavailable: {e}"),
    };
    let [l, r] = state.meter_db;
    let meter_line = format!("peak  L {} dB  R {} dB", fmt_db(l), fmt_db(r));
    let capture_line = capture_phase_text(&state.capture_phase, state.sample_rate);
    let loops_line = format!(
        "{} loop(s) · {} BPM",
        state.playing.len(),
        state.session.bpm as u32
    );

    let palette = state.palette;
    let meter_bars = flex_col((
        sized_box(meter_view(
            db_to_norm(l, METER_FLOOR_DB),
            palette.primary,
            palette.surface_2,
        ))
        .width(METER_W.px())
        .height(METER_H.px()),
        sized_box(meter_view(
            db_to_norm(r, METER_FLOOR_DB),
            palette.primary,
            palette.surface_2,
        ))
        .width(METER_W.px())
        .height(METER_H.px()),
    ))
    .gap(SP_1);

    // Surface navigation: one button per surface, the active one marked.
    let active = state.surface;
    let nav: Vec<AnyFlexChild<AppState>> = [Surface::Tracks, Surface::Combination, Surface::Settings]
        .into_iter()
        .map(|s| {
            let marker = if s == active { "● " } else { "  " };
            let lbl = format!("{}{}", marker, s.label());
            text_button(lbl, move |st: &mut AppState| st.show(s)).into_any_flex()
        })
        .collect();

    let title_row = flex_row((
        label("Strophe").text_size(TS_XL),
        flex_row(nav).gap(SP_2),
    ))
    .gap(SP_3);

    // While a free (unclocked) capture is running, Record becomes Stop.
    let record_label = if matches!(state.capture_phase, CapturePhase::FreeRecording { .. }) {
        "■ Stop recording"
    } else {
        "● Record (armed track)"
    };
    // Undo/redo labels hint availability (no-op when unavailable).
    let undo_label = if state.can_undo() { "↶ Undo" } else { "↶ —" };
    let redo_label = if state.can_redo() { "↷ Redo" } else { "↷ —" };
    let controls = flex_row((
        text_button(record_label, |st: &mut AppState| st.record()),
        text_button("■ Stop all loops", |st: &mut AppState| st.stop_all()),
        text_button(undo_label, |st: &mut AppState| st.undo()),
        text_button(redo_label, |st: &mut AppState| st.redo()),
    ))
    .gap(SP_3);

    flex_col((
        title_row,
        label(status).text_size(TS_SM),
        label(meter_line).text_size(TS_SM).font(mono_family()),
        meter_bars,
        label(capture_line).text_size(TS_SM).font(mono_family()),
        label(loops_line).text_size(TS_SM).font(mono_family()),
        controls,
    ))
    .gap(SP_2)
}

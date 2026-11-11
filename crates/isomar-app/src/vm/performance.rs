//! The Performance page: what the log holds, what it says about the instruments, the learnt polar,
//! the performance factors, how good the forecast was, waves, and passages planned against sailed.

use crate::app::AppState;
use crate::core::perf::analyse::Report;
use crate::core::perf::calibrate::Calibration;
use crate::core::perf::factors::{Factor, Factors};
use crate::core::perf::forecast_check::Lead;
use crate::core::perf::learn::{BIN_DEG, PRIOR_BLOCKS};
use crate::core::perf::{Hint, ModeSource, SailMode};
use crate::core::routing::polar::{EDITOR_TWS, Polar};
use crate::i18n::{gettext, gettext_f};
use chrono::{Local, TimeZone, Utc};
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Switch {
    /// "sailing", "motoring", or "" when not set.
    pub mode: String,
    /// An engine reports and decides; the switch cannot be changed.
    pub by_engine: bool,
    pub note: Option<String>,
    /// The boat looks motored while the switch says otherwise.
    pub hint: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Finding {
    pub title: String,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CalibrationView {
    pub findings: Vec<Finding>,
    /// What it rests on: "From 34 tacks, 12 gybes and 40 hours".
    pub evidence: String,
    /// The log's estimate differs from what is saved, so Save means something.
    pub can_save: bool,
    pub saved: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Curve {
    pub tws: f64,
    /// (angle, knots) every 5 degrees from 0 to 180: the boat's polar now, and as learnt.
    pub current: Vec<(f64, f64)>,
    pub learned: Vec<(f64, f64)>,
    /// What the log holds in this column: angle, speed (the top of what was done), confidence 0..1.
    pub cells: Vec<(f64, f64, f64)>,
    /// The best angles to windward and downwind, where the log covers them.
    pub best: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LearnedView {
    pub hours: f64,
    pub summary: String,
    pub curves: Vec<Curve>,
    /// Confidence per wind-speed column and 5 degree bin, 0..1.
    pub coverage: Vec<Vec<f64>>,
    pub columns: Vec<f64>,
    /// Cells of the boat's polar the learnt values would change at the default confidence.
    pub changes: usize,
    /// The instruments need a correction the analysis does not apply yet: the learnt polar has their errors.
    pub caution: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FactorRow {
    pub label: String,
    pub pct: f64,
    pub lo: f64,
    pub hi: f64,
    pub hours: f64,
    pub text: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FactorGroup {
    pub title: String,
    pub note: String,
    pub rows: Vec<FactorRow>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub pct: f64,
    pub text: String,
    pub button: String,
    /// Weather routing already starts from it.
    pub applied: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub title: String,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Performance {
    pub recording: bool,
    pub switch: Switch,
    pub analysing: bool,
    pub backfilling: bool,
    /// Samples a hindcast could be fetched for now.
    pub hindcast_pending: usize,
    pub empty: bool,
    pub summary: String,
    pub hours: Vec<Line>,
    pub calibration: Option<CalibrationView>,
    pub learned: Option<LearnedView>,
    pub factors: Vec<FactorGroup>,
    pub suggestion: Option<Suggestion>,
    pub forecast_check: Vec<Line>,
    pub waves: Vec<Line>,
    pub passages: Vec<Line>,
    pub routing_pct: Option<f64>,
    pub can_undo_polar: bool,
    /// The active polar's name: the log shown is the one measured against it, and learns into it.
    pub polar: String,
    pub error: Option<String>,
}

/// The confidence at which the learnt values are offered.
pub const APPLY_CONFIDENCE: f64 = 0.5;

fn h(x: f64) -> String {
    if x < 10.0 { format!("{x:.1}") } else { format!("{x:.0}") }
}

pub fn switch(app: &AppState) -> Switch {
    let s = app.session.borrow();
    let p = &s.perf;
    Switch {
        mode: p.mode.as_str().to_string(),
        by_engine: p.mode_from == ModeSource::Engine,
        note: (p.mode_from == ModeSource::Engine).then(|| gettext("Set by the engine")),
        hint: (p.hint == Hint::LooksMotored && p.mode != SailMode::Motoring && p.mode_from == ModeSource::User).then(|| gettext("The boat seems to be under engine")),
    }
}

fn calibration_view(est: &Calibration, saved: Option<&Calibration>) -> Option<CalibrationView> {
    if est.is_empty() {
        return None;
    }
    let mut findings = Vec::new();
    if let Some(o) = est.awa_offset {
        let side = if o >= 0.0 { gettext("to starboard") } else { gettext("to port") };
        findings.push(Finding {
            title: if o.abs() < 1.0 { gettext("Wind vane aligned") } else { gettext_f("Wind vane turned {deg}° {side}", &[("deg", &format!("{:.1}", o.abs())), ("side", &side)]) },
            detail: gettext("Close-hauled, the boat seems to point higher on one tack than on the other by twice this."),
        });
    }
    for (e, upwind) in [(est.angle_error[0], true), (est.angle_error[1], false)] {
        let Some(e) = e else { continue };
        let way = if e >= 0.0 { gettext("too wide") } else { gettext("too narrow") };
        findings.push(Finding {
            title: if upwind {
                gettext_f("Wind angle reads {deg}° {way} upwind", &[("deg", &format!("{:.1}", e.abs())), ("way", &way)])
            } else {
                gettext_f("Wind angle reads {deg}° {way} downwind", &[("deg", &format!("{:.1}", e.abs())), ("way", &way)])
            },
            detail: if upwind {
                gettext("The true wind seems to shift at every tack by twice this.")
            } else {
                gettext("The true wind seems to shift at every gybe by twice this.")
            },
        });
    }
    if let Some(f) = est.speed_factor {
        let pct = (f - 1.0) * 100.0;
        findings.push(Finding {
            title: if pct.abs() < 1.0 {
                gettext("Speedo reads right")
            } else if pct > 0.0 {
                gettext_f("Speedo reads {pct}% low", &[("pct", &format!("{:.0}", pct.abs() / f))])
            } else {
                gettext_f("Speedo reads {pct}% high", &[("pct", &format!("{:.0}", pct.abs() / f))])
            },
            detail: gettext("Measured against the GPS over hours with several headings, the current taken out."),
        });
    }
    let close = |a: Option<f64>, b: Option<f64>, tol: f64| match (a, b) {
        (Some(x), Some(y)) => (x - y).abs() < tol,
        (None, None) => true,
        _ => false,
    };
    let same = saved.is_some_and(|s| close(s.awa_offset, est.awa_offset, 0.5) && close(s.angle_error[0], est.angle_error[0], 0.5) && close(s.angle_error[1], est.angle_error[1], 0.5) && close(s.speed_factor, est.speed_factor, 0.01));
    Some(CalibrationView {
        findings,
        evidence: gettext_f(
            "From {tacks} tacks, {gybes} gybes, {up} h close-hauled and {speed} h of speed checks",
            &[("tacks", &est.tacks.to_string()), ("gybes", &est.gybes.to_string()), ("up", &h(est.upwind_hours)), ("speed", &est.speed_hours.to_string())],
        ),
        can_save: !same,
        saved: saved.is_some(),
    })
}

fn sweep(p: &Polar, tws: f64) -> Vec<(f64, f64)> {
    (0..=36).map(|i| (i as f64 * 5.0, (p.speed(i as f64 * 5.0, tws) * 100.0).round() / 100.0)).collect()
}

fn learned_view(r: &Report, current: &Polar) -> Option<LearnedView> {
    let l = r.learned.as_ref()?;
    let bins = (180.0 / BIN_DEG) as usize;
    let mut coverage = vec![vec![0.0; bins]; EDITOR_TWS.len()];
    for c in &l.cells {
        if let Some(j) = EDITOR_TWS.iter().position(|w| *w == c.tws) {
            let i = ((c.twa / BIN_DEG) as usize).min(bins - 1);
            coverage[j][i] = c.blocks / (c.blocks + PRIOR_BLOCKS);
        }
    }
    let curves = EDITOR_TWS
        .iter()
        .enumerate()
        .map(|(j, w)| {
            let best = match (l.best_up[j], l.best_down[j]) {
                (None, None) => None,
                (up, down) => Some(
                    [
                        up.map(|(a, v)| gettext_f("best upwind {a}° at {v} kn made good", &[("a", &format!("{a:.0}")), ("v", &format!("{v:.1}"))])),
                        down.map(|(a, v)| gettext_f("best downwind {a}° at {v} kn made good", &[("a", &format!("{a:.0}")), ("v", &format!("{v:.1}"))])),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", "),
                ),
            };
            Curve {
                tws: *w,
                current: sweep(current, *w),
                learned: sweep(&l.polar, *w),
                cells: l.cells.iter().filter(|c| c.tws == *w).map(|c| (c.twa, (c.speed * 100.0).round() / 100.0, c.blocks / (c.blocks + PRIOR_BLOCKS))).collect(),
                best,
            }
        })
        .collect();
    let mut probe = current.resampled(&EDITOR_TWS);
    let changes = crate::flows::perf::merge_learned(&mut probe, l, APPLY_CONFIDENCE);
    Some(LearnedView {
        hours: l.hours,
        summary: gettext_f("Learnt from {h} h of steady sailing with the instruments", &[("h", &h(l.hours))]),
        curves,
        coverage,
        columns: EDITOR_TWS.to_vec(),
        changes,
        caution: None,
    })
}

fn factor_row(label: String, f: Option<Factor>) -> Option<FactorRow> {
    let f = f?;
    Some(FactorRow {
        label,
        pct: (f.k * 100.0).round(),
        lo: (f.lo * 100.0).round(),
        hi: (f.hi * 100.0).round(),
        hours: f.hours,
        text: gettext_f("{pct}% of the polar ({lo} to {hi}%), {h} h", &[("pct", &format!("{:.0}", f.k * 100.0)), ("lo", &format!("{:.0}", f.lo * 100.0)), ("hi", &format!("{:.0}", f.hi * 100.0)), ("h", &h(f.hours))]),
    })
}

fn factor_group(title: String, note: String, f: &Factors) -> Option<FactorGroup> {
    let rows: Vec<FactorRow> = [
        factor_row(gettext("Overall"), f.overall),
        factor_row(gettext("Upwind"), f.upwind),
        factor_row(gettext("Reaching"), f.reach),
        factor_row(gettext("Running"), f.run),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!rows.is_empty()).then_some(FactorGroup { title, note, rows })
}

fn lead_name(l: Lead) -> String {
    match l {
        Lead::Hindcast => gettext("Analysed afterwards"),
        Lead::Day1 => gettext("Forecast up to a day old"),
        Lead::Day2 => gettext("Forecast one to two days old"),
        Lead::Older => gettext("Older forecasts"),
    }
}

fn local(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0).single().map(|t| t.with_timezone(&Local).format("%e %b %H:%M").to_string().trim().to_string()).unwrap_or_default()
}

fn duration(s: f64) -> String {
    let m = (s / 60.0).round() as i64;
    format!("{}:{:02}", m / 60, m % 60)
}

pub fn performance(app: &AppState) -> Performance {
    let settings = app.settings.borrow().clone();
    let state = app.perf.borrow();
    let mut out = Performance {
        recording: settings.perf_recording,
        switch: switch(app),
        analysing: state.analysing,
        backfilling: state.backfilling,
        hindcast_pending: crate::flows::perf::hindcast_pending(app),
        routing_pct: crate::polars::performance_pct(app),
        can_undo_polar: crate::polars::library().has_previous(&crate::polars::active_id(app)),
        polar: crate::polars::active_name(app),
        error: state.error.clone(),
        ..Default::default()
    };
    out.passages = state
        .passages
        .iter()
        .map(|p| Line {
            title: p.name.clone(),
            detail: gettext_f(
                "{when}: planned {planned}, sailed in {sailed} ({pct}% of the planned speed), {motored}% under engine",
                &[
                    ("when", &local(p.finished)),
                    ("planned", &duration(p.planned_s)),
                    ("sailed", &duration(p.sailed_s)),
                    ("pct", &format!("{:.0}", p.ratio() * 100.0)),
                    ("motored", &format!("{:.0}", p.motored * 100.0)),
                ],
            ),
        })
        .collect();
    let Some(r) = state.report.as_ref() else {
        out.empty = true;
        out.summary = if state.analysing { gettext("Analysing the log…") } else { gettext("Nothing logged yet. The log fills while the boat is under way.") };
        return out;
    };
    if r.samples == 0 {
        out.empty = true;
        out.summary = gettext("Nothing logged yet. The log fills while the boat is under way.");
        return out;
    }
    let since = r.from.map(|t| t.with_timezone(&Local).format("%e %B %Y").to_string().trim().to_string()).unwrap_or_default();
    out.summary = gettext_f("{h} h logged since {since}", &[("h", &h(r.hours.total)), ("since", &since)]);
    let hours = &r.hours;
    out.hours = [
        (gettext("Sailing"), hours.sailing),
        (gettext("Motoring"), hours.motoring),
        (gettext("Not set"), hours.unset),
        (gettext("Looked motored"), hours.looked_motored),
        (gettext("Steady sailing with wind instruments and speedo"), hours.tier_a),
        (gettext("Steady sailing with wind instruments, no speedo"), hours.tier_c),
        (gettext("Steady sailing with the forecast's wind only"), hours.tier_d),
    ]
    .into_iter()
    .filter(|(_, x)| *x >= 0.05)
    .map(|(t, x)| Line { title: t, detail: gettext_f("{h} h", &[("h", &h(x))]) })
    .collect();
    let current = crate::polars::active(app);
    out.calibration = calibration_view(&r.estimate, settings.perf_calibration.as_ref());
    out.learned = learned_view(r, &current);
    // A correction worth making and not applied: the learnt polar is the instruments' errors as much as the boat.
    let e = &r.estimate;
    let significant = e.awa_offset.is_some_and(|o| o.abs() >= 1.0) || e.angle_error.iter().flatten().any(|x| x.abs() >= 1.0) || e.speed_factor.is_some_and(|f| (f - 1.0).abs() >= 0.01);
    if significant && out.calibration.as_ref().is_some_and(|c| c.can_save)
        && let Some(l) = out.learned.as_mut()
    {
        l.caution = Some(gettext("The instruments need the correction above, which this analysis does not apply yet: the learnt polar carries their errors. Correct the analysis with it first."));
    }
    out.factors = [
        factor_group(gettext("With the instruments' wind"), gettext("Speed through the water against the polar, at the wind the instruments measured."), &r.instruments),
        factor_group(
            gettext("With the forecast's wind"),
            gettext("A forecast is often 25° and 30% off, too far to learn the polar from: it only says how fast the boat is overall. The polar is blurred by the same errors before the comparison."),
            &r.forecast,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    out.suggestion = r.suggestion.map(|s| Suggestion {
        pct: s.pct,
        text: if s.from == "instruments" {
            gettext_f("The boat sails at about {pct}% of its polar, by the instruments.", &[("pct", &format!("{:.0}", s.pct))])
        } else {
            gettext_f("The boat sails at about {pct}% of its polar, by the forecast's wind.", &[("pct", &format!("{:.0}", s.pct))])
        },
        button: gettext_f("Use {pct}% for weather routing", &[("pct", &format!("{:.0}", s.pct))]),
        applied: crate::polars::performance_pct(app) == Some(s.pct),
    });
    out.forecast_check = r
        .forecast_check
        .iter()
        .filter(|c| c.hours >= 0.5)
        .map(|c| Line {
            title: lead_name(c.lead),
            detail: gettext_f(
                "Wind speed {bias} kn on average (± {rms} kn), direction ± {dir}°, over {h} h",
                &[("bias", &format!("{:+.1}", c.tws_bias)), ("rms", &format!("{:.1}", c.tws_rms)), ("dir", &format!("{:.0}", c.twd_rms)), ("h", &h(c.hours))],
            ),
        })
        .collect();
    out.waves = r
        .waves
        .iter()
        .map(|w| Line {
            title: if w.hi >= f64::MAX / 2.0 {
                if w.upwind { gettext_f("Upwind, waves over {lo} m", &[("lo", &format!("{:.0}", w.lo))]) } else { gettext_f("Downwind, waves over {lo} m", &[("lo", &format!("{:.0}", w.lo))]) }
            } else if w.upwind {
                gettext_f("Upwind, waves {lo} to {hi} m", &[("lo", &format!("{:.0}", w.lo)), ("hi", &format!("{:.0}", w.hi))])
            } else {
                gettext_f("Downwind, waves {lo} to {hi} m", &[("lo", &format!("{:.0}", w.lo)), ("hi", &format!("{:.0}", w.hi))])
            },
            detail: gettext_f("{pct}% of the polar, {h} h", &[("pct", &format!("{:.0}", w.ratio * 100.0)), ("h", &h(w.hours))]),
        })
        .collect();
    out
}

/// What the polar editor overlays for one wind-speed column set: the learnt polar with its confidence.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Observed {
    pub tws: Vec<f64>,
    pub twa: Vec<f64>,
    pub speeds: Vec<Vec<f64>>,
    pub confidence: Vec<Vec<f64>>,
    /// Per column of `tws`: (angle, speed, confidence) of the bins the log holds.
    pub cells: Vec<Vec<(f64, f64, f64)>>,
}

pub fn observed(app: &AppState) -> Option<Observed> {
    let state = app.perf.borrow();
    let l = state.report.as_ref()?.learned.as_ref()?;
    Some(Observed {
        tws: l.polar.tws.clone(),
        twa: l.polar.twa.clone(),
        speeds: l.polar.speeds.clone(),
        confidence: l.confidence.clone(),
        cells: l.polar.tws.iter().map(|w| l.cells.iter().filter(|c| c.tws == *w).map(|c| (c.twa, c.speed, c.blocks / (c.blocks + PRIOR_BLOCKS))).collect()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::nav::store::NavStore;
    use crate::core::perf::synth::{self, Config, scaled};
    use crate::core::session::Session;
    use crate::engine::Engine;
    use crate::palette::Mode;
    use std::rc::Rc;

    fn app() -> Rc<AppState> {
        let canvas = Engine::new(crate::config::ViewState { lat: 48.0, lon: -4.0, zoom: 10.0 }, Mode::Day);
        AppState::new(canvas, Session::new(NavStore::open_memory().unwrap()).unwrap(), crate::config::Settings::default())
    }

    #[test]
    fn an_empty_log_says_so() {
        let a = app();
        let p = performance(&a);
        assert!(p.empty && p.recording && p.calibration.is_none());
        assert_eq!(p.switch.mode, "");
    }

    #[test]
    fn a_season_fills_every_group() {
        let a = app();
        let reference = Polar::generic_cruiser();
        let log = synth::log(&Config { hours: 80.0, truth: scaled(&reference, 0.9, 1.0), awa_offset: 3.0, ..Default::default() });
        {
            let s = a.session.borrow();
            for x in &log {
                s.store.insert_perf_sample(x).unwrap();
            }
        }
        let r = crate::core::perf::analyse::analyse(&log, &reference, None, Utc::now());
        a.perf.borrow_mut().report = Some(Rc::new(r));
        let p = performance(&a);
        assert!(!p.empty, "{p:?}");
        assert!(p.summary.contains("80"), "{}", p.summary);
        let cal = p.calibration.unwrap();
        assert!(cal.can_save && cal.findings.iter().any(|f| f.title.contains("turned")), "{cal:?}");
        let l = p.learned.unwrap();
        assert_eq!(l.curves.len(), EDITOR_TWS.len());
        assert!(l.changes > 0 && l.coverage.iter().flatten().any(|c| *c > 0.5));
        assert_eq!(p.factors.len(), 2);
        let s = p.suggestion.unwrap();
        assert!(!s.applied && s.button.contains('%'));
        assert!(!p.forecast_check.is_empty() && !p.waves.is_empty());
        assert!(observed(&a).is_some());
        let json = serde_json::to_value(performance(&a)).unwrap();
        assert!(json["learned"]["curves"][0]["current"].is_array());
    }
}

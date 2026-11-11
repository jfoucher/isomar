//! Magnetic variation under the pointer: what a chart cell in use says (its `MAGVAR`, brought up to date from the
//! year it was measured by the yearly change it gives), else the World Magnetic Model, which has it anywhere.

use super::app::AppState;
use crate::core::geo::LatLon;
use crate::core::geomag;
use crate::i18n::gettext_f;

/// Variation in degrees, east positive, and its yearly change in minutes of arc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variation {
    pub degrees: f64,
    pub annual_arcmin: Option<f64>,
    /// Stated by a chart cell, not worked out from the model.
    pub from_chart: bool,
}

/// What a chart cell states, brought to `year`.
pub fn from_chart(c: &crate::layers::enc::ChartVariation, year: f64) -> f64 {
    match (c.annual_change, c.year) {
        (Some(per_year), Some(measured)) => c.value + per_year / 60.0 * (year - measured),
        _ => c.value,
    }
}

/// The variation at `p` today.
pub fn at(app: &AppState, p: &LatLon) -> Option<Variation> {
    let year = geomag::today();
    let chart = app.enc.borrow().as_ref().and_then(|e| e.variation_at(p.lon, p.lat));
    if let Some(c) = chart {
        return Some(Variation { degrees: from_chart(&c, year), annual_arcmin: c.annual_change, from_chart: true });
    }
    let degrees = geomag::variation(p.lat, p.lon, year)?;
    Some(Variation { degrees, annual_arcmin: geomag::annual_change(p.lat, p.lon, year).map(|d| d * 60.0), from_chart: false })
}

/// Degrees and minutes with the hemisphere: "1°12′W".
pub fn degrees_text(deg: f64) -> String {
    let minutes = (deg.abs() * 60.0).round() as i64;
    let side = if deg < 0.0 && minutes != 0 { "W" } else if minutes != 0 { "E" } else { "" };
    format!("{}°{:02}′{side}", minutes / 60, minutes % 60)
}

/// "1°12′W (+4′/yr)": the variation without its name, to stand beside an icon.
pub fn value_text(v: &Variation) -> String {
    let now = degrees_text(v.degrees);
    match v.annual_arcmin.filter(|a| a.abs() >= 0.5) {
        Some(a) => format!("{now} ({:+.0}′/yr)", a),
        None => now,
    }
}

/// "Var 1°12′W (+4′/yr)": the variation, and the way it is changing where that is known.
pub fn text(v: &Variation) -> String {
    let now = degrees_text(v.degrees);
    match v.annual_arcmin.filter(|a| a.abs() >= 0.5) {
        Some(a) => gettext_f("Var {variation} ({change}′/yr)", &[("variation", &now), ("change", &format!("{a:+.0}"))]),
        None => gettext_f("Var {variation}", &[("variation", &now)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::enc::ChartVariation;

    #[test]
    fn degrees_are_written_with_their_hemisphere() {
        assert_eq!(degrees_text(-1.2), "1°12′W");
        assert_eq!(degrees_text(2.5), "2°30′E");
        assert_eq!(degrees_text(0.001), "0°00′");
        assert_eq!(degrees_text(-12.9833), "12°59′W");
    }

    #[test]
    fn a_charts_value_is_brought_up_to_date_by_its_yearly_change() {
        let c = ChartVariation { value: -4.0, annual_change: Some(6.0), year: Some(2000.0) };
        assert!((from_chart(&c, 2010.0) - -3.0).abs() < 1e-9, "6 minutes a year for 10 years is a degree");
        let c = ChartVariation { value: -4.0, annual_change: None, year: Some(2000.0) };
        assert_eq!(from_chart(&c, 2026.0), -4.0);
    }

    #[test]
    fn the_text_says_which_way_it_is_changing() {
        assert_eq!(text(&Variation { degrees: -1.2, annual_arcmin: Some(4.0), from_chart: true }), "Var 1°12′W (+4′/yr)");
        assert_eq!(text(&Variation { degrees: 2.0, annual_arcmin: None, from_chart: false }), "Var 2°00′E");
    }
}

//! Boat polars: boat speed as a function of true wind angle and speed.

#[derive(Debug, PartialEq, Eq)]
pub enum PolarError {
    Empty,
    BadHeader,
    BadNumber(String),
    NotIncreasing(&'static str),
}

impl std::fmt::Display for PolarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolarError::Empty => write!(f, "the polar file is empty"),
            PolarError::BadHeader => write!(f, "the first line must list the wind speeds"),
            PolarError::BadNumber(s) => write!(f, "cannot read {s:?} as a number"),
            PolarError::NotIncreasing(what) => write!(f, "the {what} must increase down and across the table"),
        }
    }
}

impl std::error::Error for PolarError {}

/// The wind speeds the polar editor offers, knots: steps of 5 up to 30, then 10.
pub const EDITOR_TWS: [f64; 7] = [5.0, 10.0, 15.0, 20.0, 25.0, 30.0, 40.0];

#[derive(Clone, Debug, PartialEq)]
pub struct Polar {
    /// True wind speeds of the columns, knots, increasing.
    pub tws: Vec<f64>,
    /// True wind angles of the rows, degrees from the bow (0..180), increasing.
    pub twa: Vec<f64>,
    /// `speeds[row][col]` in knots.
    pub speeds: Vec<Vec<f64>>,
}

impl Polar {
    /// Reads the common text form: a header row of wind speeds (its first
    /// cell, such as `TWA/TWS`, is ignored), then one row per angle starting
    /// with the angle. Separators may be `;`, `,`, tab or spaces.
    pub fn parse(text: &str) -> Result<Polar, PolarError> {
        let mut rows: Vec<Vec<String>> = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cells: Vec<String> = if line.contains(';') {
                line.split(';').map(|s| s.trim().to_string()).collect()
            } else if line.contains('\t') {
                line.split('\t').map(|s| s.trim().to_string()).collect()
            } else if line.contains(',') && !line.contains(", ") && line.matches(',').count() > 2 {
                line.split(',').map(|s| s.trim().to_string()).collect()
            } else {
                line.split_whitespace().map(|s| s.to_string()).collect()
            };
            rows.push(cells.into_iter().filter(|c| !c.is_empty()).collect());
        }
        let Some(header) = rows.first() else { return Err(PolarError::Empty) };
        if header.len() < 2 {
            return Err(PolarError::BadHeader);
        }
        let num = |s: &String| s.replace(',', ".").parse::<f64>().map_err(|_| PolarError::BadNumber(s.clone()));
        let tws: Vec<f64> = header[1..].iter().map(num).collect::<Result<_, _>>()?;
        if tws.windows(2).any(|w| w[1] <= w[0]) {
            return Err(PolarError::NotIncreasing("wind speeds"));
        }
        let mut twa = Vec::new();
        let mut speeds = Vec::new();
        for r in &rows[1..] {
            if r.len() < 2 {
                continue;
            }
            twa.push(num(&r[0])?);
            let mut row: Vec<f64> = r[1..].iter().map(num).collect::<Result<_, _>>()?;
            row.resize(tws.len(), 0.0);
            speeds.push(row);
        }
        if twa.is_empty() {
            return Err(PolarError::Empty);
        }
        if twa.windows(2).any(|w| w[1] <= w[0]) {
            return Err(PolarError::NotIncreasing("wind angles"));
        }
        Ok(Polar { tws, twa, speeds })
    }

    /// A plausible polar for a 10 m cruising sloop, for the tests and the routing bench's references.
    /// The numbers are generic, not measured; the app's polars are in `routing::library`.
    pub fn generic_cruiser() -> Polar {
        Polar::parse(
            "TWA/TWS;4;6;8;10;12;14;16;20;25
             0;0;0;0;0;0;0;0;0;0
             40;2.4;3.6;4.5;5.1;5.5;5.7;5.8;5.8;5.5
             52;3.0;4.4;5.4;6.0;6.4;6.6;6.7;6.7;6.4
             60;3.3;4.8;5.8;6.4;6.8;7.0;7.1;7.1;6.9
             75;3.6;5.1;6.1;6.7;7.1;7.4;7.5;7.6;7.4
             90;3.7;5.2;6.2;6.9;7.3;7.7;7.9;8.1;8.0
             110;3.6;5.2;6.3;7.0;7.5;8.0;8.3;8.7;8.7
             135;3.1;4.6;5.7;6.5;7.1;7.6;8.0;8.7;9.0
             150;2.7;4.0;5.0;5.8;6.4;6.9;7.4;8.2;8.7
             180;2.3;3.5;4.4;5.1;5.7;6.2;6.6;7.4;8.0",
        )
        .expect("the built-in polar is valid")
    }

    /// The table in the text form [`Polar::parse`] reads back.
    pub fn to_text(&self) -> String {
        let num = |v: f64| format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string();
        let mut out = String::from("TWA/TWS");
        for w in &self.tws {
            out.push(';');
            out.push_str(&num(*w));
        }
        out.push('\n');
        for (a, row) in self.twa.iter().zip(&self.speeds) {
            out.push_str(&num(*a));
            for s in row {
                out.push(';');
                out.push_str(&num(*s));
            }
            out.push('\n');
        }
        out
    }

    /// The same boat on a different set of wind speeds, read off this one.
    /// `tws` must increase.
    pub fn resampled(&self, tws: &[f64]) -> Polar {
        let speeds = self.twa.iter().map(|a| tws.iter().map(|w| (self.speed(*a, *w) * 100.0).round() / 100.0).collect()).collect();
        Polar { tws: tws.to_vec(), twa: self.twa.clone(), speeds }
    }

    /// Sets the speed at one angle and wind speed of the table (negative
    /// values become 0). Out-of-range indices are ignored.
    pub fn set_speed(&mut self, row: usize, col: usize, knots: f64) {
        if let Some(cell) = self.speeds.get_mut(row).and_then(|r| r.get_mut(col)) {
            *cell = knots.max(0.0);
        }
    }

    /// The fastest the polar gives, at any angle and wind speed, in knots.
    pub fn max_speed(&self) -> f64 {
        self.speeds.iter().flatten().copied().fold(0.0, f64::max)
    }

    /// The speed of each row of the table at `tws`, so that `AtWind::speed` gives `speed(twa, tws)` for
    /// many angles without working out the wind speed's place in the table for each (the router tries 72
    /// headings in one wind).
    pub fn at_wind(&self, tws: f64) -> AtWind<'_> {
        let tws = tws.max(0.0);
        AtWind { twa: &self.twa, speeds: (0..self.twa.len()).map(|row| self.row_speed(row, tws)).collect() }
    }

    /// Row `row` of the table at `tws` (not negative): see `speed`.
    fn row_speed(&self, row: usize, tws: f64) -> f64 {
        let r = &self.speeds[row];
        if tws <= self.tws[0] {
            return r[0] * tws / self.tws[0].max(1e-9);
        }
        let (j0, j1, tw) = bracket(&self.tws, tws);
        r[j0] + (r[j1] - r[j0]) * tw
    }

    /// Boat speed in knots. `twa` may be any angle (it is folded to 0..180).
    /// Beyond the table's wind speeds the last column is used; below its
    /// first the speed falls linearly to nothing in no wind. Angles beyond
    /// the table's ends use the nearest row.
    pub fn speed(&self, twa: f64, tws: f64) -> f64 {
        let a = {
            let m = twa.rem_euclid(360.0);
            if m > 180.0 { 360.0 - m } else { m }
        };
        let tws = tws.max(0.0);
        let (i0, i1, ta) = bracket(&self.twa, a);
        let col = |row: usize| self.row_speed(row, tws);
        col(i0) + (col(i1) - col(i0)) * ta
    }

    /// The best angle and speed made good to windward or downwind at a wind speed.
    pub fn best_vmg(&self, tws: f64, upwind: bool) -> (f64, f64) {
        let mut best = (0.0, 0.0);
        let mut a = 0.0;
        while a <= 180.0 {
            let vmg = self.speed(a, tws) * if upwind { a.to_radians().cos() } else { -a.to_radians().cos() };
            if vmg > best.1 {
                best = (a, vmg);
            }
            a += 1.0;
        }
        best
    }
}

/// The two table indices around `x` and the weight of the second.
/// A polar at one wind speed (`Polar::at_wind`).
pub struct AtWind<'a> {
    twa: &'a [f64],
    speeds: Vec<f64>,
}

impl AtWind<'_> {
    /// `Polar::speed(twa, tws)` for the wind speed this was made for, exactly.
    pub fn speed(&self, twa: f64) -> f64 {
        let a = {
            let m = twa.rem_euclid(360.0);
            if m > 180.0 { 360.0 - m } else { m }
        };
        let (i0, i1, ta) = bracket(self.twa, a);
        self.speeds[i0] + (self.speeds[i1] - self.speeds[i0]) * ta
    }
}

fn bracket(axis: &[f64], x: f64) -> (usize, usize, f64) {
    if x <= axis[0] {
        return (0, 0, 0.0);
    }
    let last = axis.len() - 1;
    if x >= axis[last] {
        return (last, last, 0.0);
    }
    let i = axis.partition_point(|v| *v <= x);
    (i - 1, i, (x - axis[i - 1]) / (axis[i] - axis[i - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_polar_at_one_wind_speed_gives_the_same_speeds() {
        let p = Polar::generic_cruiser();
        for tws in [0.0, 1.5, 4.0, 6.0, 9.3, 14.0, 22.7, 40.0, 60.0] {
            let at = p.at_wind(tws);
            for twa in [-30.0, 0.0, 12.0, 37.5, 45.0, 90.0, 133.3, 180.0, 210.0, 359.0, 725.0] {
                assert_eq!(at.speed(twa), p.speed(twa, tws), "{twa} in {tws} kn");
            }
        }
    }

    const TABLE: &str = "TWA/TWS;6;10\n0;0;0\n90;5;8\n180;3;5\n";

    #[test]
    fn parses_semicolon_tables() {
        let p = Polar::parse(TABLE).unwrap();
        assert_eq!(p.tws, vec![6.0, 10.0]);
        assert_eq!(p.twa, vec![0.0, 90.0, 180.0]);
        assert_eq!(p.speeds[1], vec![5.0, 8.0]);
    }

    #[test]
    fn parses_tabs_spaces_and_comments() {
        let p = Polar::parse("# my boat\nTWA\\TWS\t6\t10\n0\t0\t0\n90\t5\t8\n").unwrap();
        assert_eq!(p.twa, vec![0.0, 90.0]);
        let p = Polar::parse("twa 6 10\n0 0 0\n 90 5 8\n").unwrap();
        assert_eq!(p.speeds[1], vec![5.0, 8.0]);
        let p = Polar::parse("TWA,6,10,14\n0,0,0,0\n90,5,8,9\n").unwrap();
        assert_eq!(p.tws.len(), 3);
    }

    #[test]
    fn short_rows_are_padded_and_errors_are_specific() {
        let p = Polar::parse("TWA;6;10\n0;0\n90;5;8\n").unwrap();
        assert_eq!(p.speeds[0], vec![0.0, 0.0]);
        assert_eq!(Polar::parse(""), Err(PolarError::Empty));
        assert_eq!(Polar::parse("TWA;6;10\n"), Err(PolarError::Empty));
        assert_eq!(Polar::parse("TWA\n0;1"), Err(PolarError::BadHeader));
        assert!(matches!(Polar::parse("TWA;6;x\n0;1;2"), Err(PolarError::BadNumber(_))));
        assert_eq!(Polar::parse("TWA;10;6\n0;1;2"), Err(PolarError::NotIncreasing("wind speeds")));
        assert_eq!(Polar::parse("TWA;6\n90;1\n45;2"), Err(PolarError::NotIncreasing("wind angles")));
    }

    #[test]
    fn interpolates_between_angles_and_wind_speeds() {
        let p = Polar::parse(TABLE).unwrap();
        assert_eq!(p.speed(90.0, 6.0), 5.0);
        assert_eq!(p.speed(90.0, 10.0), 8.0);
        assert!((p.speed(90.0, 8.0) - 6.5).abs() < 1e-9);
        assert!((p.speed(45.0, 6.0) - 2.5).abs() < 1e-9, "halfway between 0 and 5");
        assert!((p.speed(135.0, 10.0) - 6.5).abs() < 1e-9);
    }

    #[test]
    fn angles_fold_and_wind_speed_clamps() {
        let p = Polar::parse(TABLE).unwrap();
        assert_eq!(p.speed(-90.0, 6.0), p.speed(90.0, 6.0));
        assert_eq!(p.speed(270.0, 6.0), p.speed(90.0, 6.0));
        assert_eq!(p.speed(450.0, 6.0), p.speed(90.0, 6.0));
        // More wind than the table: the last column.
        assert_eq!(p.speed(90.0, 40.0), 8.0);
        // Less wind: falls to zero.
        assert!((p.speed(90.0, 3.0) - 2.5).abs() < 1e-9);
        assert_eq!(p.speed(90.0, 0.0), 0.0);
        assert_eq!(p.speed(90.0, -5.0), 0.0);
    }

    #[test]
    fn text_round_trips() {
        let p = Polar::generic_cruiser();
        assert_eq!(Polar::parse(&p.to_text()).unwrap(), p);
        let q = p.resampled(&EDITOR_TWS);
        assert_eq!(Polar::parse(&q.to_text()).unwrap(), q);
    }

    #[test]
    fn resampling_keeps_the_boat_and_editing_changes_one_cell() {
        let p = Polar::generic_cruiser();
        let mut q = p.resampled(&EDITOR_TWS);
        assert_eq!(q.tws, EDITOR_TWS.to_vec());
        assert_eq!(q.twa, p.twa);
        // 10 kn is a column of the generic table: unchanged.
        assert!((q.speed(90.0, 10.0) - p.speed(90.0, 10.0)).abs() < 0.01);
        let (row, col) = (q.twa.iter().position(|a| *a == 90.0).unwrap(), 1);
        let before = q.speeds.clone();
        q.set_speed(row, col, 9.5);
        assert_eq!(q.speed(90.0, 10.0), 9.5);
        q.set_speed(row, col, -3.0);
        assert_eq!(q.speeds[row][col], 0.0);
        q.set_speed(99, 99, 1.0);
        q.set_speed(row, col, before[row][col]);
        assert_eq!(q.speeds, before);
    }

    #[test]
    fn the_generic_polar_behaves_like_a_sailboat() {
        let p = Polar::generic_cruiser();
        assert_eq!(p.speed(0.0, 12.0), 0.0, "cannot sail into the wind");
        assert!(p.speed(90.0, 12.0) > p.speed(40.0, 12.0));
        assert!(p.speed(120.0, 16.0) > p.speed(90.0, 8.0));
        let (up_angle, up_vmg) = p.best_vmg(12.0, true);
        assert!((38.0..60.0).contains(&up_angle), "{up_angle}");
        assert!((2.0..5.0).contains(&up_vmg), "{up_vmg}");
        let (down_angle, down_vmg) = p.best_vmg(12.0, false);
        assert!(down_angle > 120.0 && down_vmg > 4.0, "{down_angle} {down_vmg}");
    }
}

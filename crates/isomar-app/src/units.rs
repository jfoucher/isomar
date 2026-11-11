//! The units the user chose for speed, distance and depth. The engine works in knots, nautical
//! miles and metres; this turns them into what is shown.

use crate::config::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Knots,
    KmH,
    MS,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distance {
    NauticalMiles,
    Kilometres,
    Miles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Depth {
    Metres,
    Feet,
    Fathoms,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Units {
    pub speed: Speed,
    pub distance: Distance,
    pub depth: Depth,
}

impl Units {
    pub fn of(s: &Settings) -> Units {
        Units {
            speed: match s.speed_unit.as_str() {
                "km/h" => Speed::KmH,
                "m/s" => Speed::MS,
                _ => Speed::Knots,
            },
            distance: match s.distance_unit.as_str() {
                "km" => Distance::Kilometres,
                "mi" => Distance::Miles,
                _ => Distance::NauticalMiles,
            },
            depth: match s.depth_unit.as_str() {
                "ft" => Depth::Feet,
                "fa" => Depth::Fathoms,
                _ => Depth::Metres,
            },
        }
    }

    /// A speed in knots as (value, unit).
    pub fn speed(&self, kn: f64) -> (f64, &'static str) {
        match self.speed {
            Speed::Knots => (kn, "kn"),
            Speed::KmH => (kn * 1.852, "km/h"),
            Speed::MS => (kn * 0.514_444, "m/s"),
        }
    }

    /// A distance in nautical miles as (value, unit).
    pub fn distance(&self, nm: f64) -> (f64, &'static str) {
        match self.distance {
            Distance::NauticalMiles => (nm, "nm"),
            Distance::Kilometres => (nm * 1.852, "km"),
            Distance::Miles => (nm * 1.150_78, "mi"),
        }
    }

    /// A depth in metres as (value, unit).
    pub fn depth(&self, m: f64) -> (f64, &'static str) {
        match self.depth {
            Depth::Metres => (m, "m"),
            Depth::Feet => (m * 3.280_84, "ft"),
            Depth::Fathoms => (m * 0.546_807, "fm"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_follow_the_settings_and_convert() {
        let mut s = Settings::default();
        let u = Units::of(&s);
        assert_eq!((u.speed(6.0), u.distance(2.0), u.depth(10.0)), ((6.0, "kn"), (2.0, "nm"), (10.0, "m")));
        s.speed_unit = "km/h".into();
        s.distance_unit = "km".into();
        s.depth_unit = "ft".into();
        let u = Units::of(&s);
        assert!((u.speed(10.0).0 - 18.52).abs() < 1e-9 && u.speed(10.0).1 == "km/h");
        assert!((u.distance(1.0).0 - 1.852).abs() < 1e-9);
        assert!((u.depth(10.0).0 - 32.8084).abs() < 1e-3 && u.depth(10.0).1 == "ft");
        s.depth_unit = "nonsense".into();
        assert_eq!(Units::of(&s).depth, Depth::Metres, "an unknown unit is the default");
    }
}

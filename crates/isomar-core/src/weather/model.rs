//! The weather parameters the app knows about.

use chrono::{DateTime, Utc};
use crate::chart::grid::Grid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Param {
    /// 10 m wind, eastward component, m/s.
    WindU,
    /// 10 m wind, northward component, m/s.
    WindV,
    /// 10 m wind gust, m/s.
    Gust,
    /// Pressure at mean sea level, Pa.
    Pressure,
    /// 2 m air temperature, K.
    Temperature,
    /// Total cloud cover, percent.
    CloudCover,
    /// Significant height of combined wind waves and swell, m.
    WaveHeight,
    /// Primary wave mean period, s.
    WavePeriod,
    /// Primary wave direction, degrees (from).
    WaveDir,
    /// Significant height of wind waves, m.
    WindWaveHeight,
    /// Significant height of swell, m.
    SwellHeight,
    /// Surface current, eastward, m/s.
    CurrentU,
    /// Surface current, northward, m/s.
    CurrentV,
    /// The surface current without its tide (Copernicus SMOC's `uo`), eastward, m/s: what a tidal
    /// atlas's stream is added to. Local-use GRIB2 numbers, in Isomar's own cache files only.
    ResidualU,
    /// The same, northward.
    ResidualV,
}

impl Param {
    pub const ALL: [Param; 15] = [
        Param::WindU,
        Param::WindV,
        Param::Gust,
        Param::Pressure,
        Param::Temperature,
        Param::CloudCover,
        Param::WaveHeight,
        Param::WavePeriod,
        Param::WaveDir,
        Param::WindWaveHeight,
        Param::SwellHeight,
        Param::CurrentU,
        Param::CurrentV,
        Param::ResidualU,
        Param::ResidualV,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Param::WindU => "wind_u",
            Param::WindV => "wind_v",
            Param::Gust => "gust",
            Param::Pressure => "pressure",
            Param::Temperature => "temperature",
            Param::CloudCover => "cloud",
            Param::WaveHeight => "wave_height",
            Param::WavePeriod => "wave_period",
            Param::WaveDir => "wave_dir",
            Param::WindWaveHeight => "windwave_height",
            Param::SwellHeight => "swell_height",
            Param::CurrentU => "current_u",
            Param::CurrentV => "current_v",
            Param::ResidualU => "residual_u",
            Param::ResidualV => "residual_v",
        }
    }

    pub fn from_id(s: &str) -> Option<Param> {
        Param::ALL.into_iter().find(|p| p.id() == s)
    }

    /// The WMO codes a field of this parameter is written with (`gribwrite`): discipline, category,
    /// number, and the first fixed surface's type and value. `from_grib` reads them back.
    pub fn grib_codes(self) -> (u8, u8, u8, u8, u32) {
        match self {
            Param::WindU => (0, 2, 2, 103, 10),
            Param::WindV => (0, 2, 3, 103, 10),
            Param::Gust => (0, 2, 22, 103, 10),
            Param::Pressure => (0, 3, 1, 101, 0),
            Param::Temperature => (0, 0, 0, 103, 2),
            Param::CloudCover => (0, 6, 1, 10, 0),
            Param::WaveHeight => (10, 0, 3, 1, 0),
            Param::WavePeriod => (10, 0, 11, 1, 0),
            Param::WaveDir => (10, 0, 10, 1, 0),
            Param::WindWaveHeight => (10, 0, 5, 1, 0),
            Param::SwellHeight => (10, 0, 8, 1, 0),
            Param::CurrentU => (10, 1, 2, 1, 0),
            Param::CurrentV => (10, 1, 3, 1, 0),
            Param::ResidualU => (10, 1, 192, 1, 0),
            Param::ResidualV => (10, 1, 193, 1, 0),
        }
    }

    /// Decimal digits worth keeping when a field is written again (a cropped download): a hundredth
    /// of a metre a second, a pascal, a tenth of a degree.
    pub fn decimals(self) -> i16 {
        match self {
            Param::Pressure | Param::CloudCover => 0,
            Param::WaveDir => 1,
            Param::CurrentU | Param::CurrentV | Param::ResidualU | Param::ResidualV => 3,
            _ => 2,
        }
    }

    /// Identifies a GRIB2 field from its WMO codes: discipline, parameter
    /// category and number, and the first fixed surface (type and value).
    pub fn from_grib(discipline: u8, category: u8, number: u8, surface: Option<(u8, f64)>) -> Option<Param> {
        let surf_type = surface.map(|s| s.0);
        let surf_value = surface.map(|s| s.1);
        let at_10m = surf_type == Some(103) && surf_value.is_some_and(|v| (v - 10.0).abs() < 0.5);
        let at_2m = surf_type == Some(103) && surf_value.is_some_and(|v| (v - 2.0).abs() < 0.5);
        match (discipline, category, number) {
            (0, 2, 2) if at_10m => Some(Param::WindU),
            (0, 2, 3) if at_10m => Some(Param::WindV),
            // Gusts are reported at the surface (GFS) or 10 m (others).
            (0, 2, 22) if at_10m || surf_type == Some(1) => Some(Param::Gust),
            (0, 3, 0) | (0, 3, 1) if surf_type == Some(101) => Some(Param::Pressure),
            (0, 0, 0) if at_2m => Some(Param::Temperature),
            (0, 6, 1) => Some(Param::CloudCover),
            (10, 0, 3) => Some(Param::WaveHeight),
            (10, 0, 10) | (10, 0, 14) => Some(Param::WaveDir),
            (10, 0, 11) | (10, 0, 15) => Some(Param::WavePeriod),
            (10, 0, 5) => Some(Param::WindWaveHeight),
            (10, 0, 8) => Some(Param::SwellHeight),
            (10, 1, 2) => Some(Param::CurrentU),
            (10, 1, 3) => Some(Param::CurrentV),
            (10, 1, 192) => Some(Param::ResidualU),
            (10, 1, 193) => Some(Param::ResidualV),
            _ => None,
        }
    }
}

/// The user-facing groups of parameters, as the download dialog offers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Wind,
    Gust,
    Pressure,
    Waves,
    Temperature,
    Cloud,
    Current,
}

impl Group {
    pub const ALL: [Group; 7] = [Group::Wind, Group::Gust, Group::Pressure, Group::Waves, Group::Temperature, Group::Cloud, Group::Current];

    pub fn params(self) -> &'static [Param] {
        match self {
            Group::Wind => &[Param::WindU, Param::WindV],
            Group::Gust => &[Param::Gust],
            Group::Pressure => &[Param::Pressure],
            Group::Waves => &[Param::WaveHeight, Param::WavePeriod, Param::WaveDir],
            Group::Temperature => &[Param::Temperature],
            Group::Cloud => &[Param::CloudCover],
            Group::Current => &[Param::CurrentU, Param::CurrentV],
        }
    }
}

/// One parameter on a regular grid at one valid time.
#[derive(Clone, Debug)]
pub struct Field {
    pub param: Param,
    /// The time the values are valid for.
    pub time: DateTime<Utc>,
    /// The model run that produced them, when known.
    pub run: Option<DateTime<Utc>>,
    /// Values in the units of the source (SI). Rows run north to south.
    pub grid: Grid,
}

/// Unit conversions for display.
pub mod units {
    pub const MS_TO_KN: f32 = 1.0 / 0.514_444;

    pub fn ms_to_kn(v: f32) -> f32 {
        v * MS_TO_KN
    }

    pub fn pa_to_hpa(v: f32) -> f32 {
        v / 100.0
    }

    pub fn k_to_c(v: f32) -> f32 {
        v - 273.15
    }

    /// Beaufort force for a wind speed in knots.
    pub fn beaufort(kn: f32) -> u8 {
        const LIMITS: [f32; 12] = [1.0, 4.0, 7.0, 11.0, 17.0, 22.0, 28.0, 34.0, 41.0, 48.0, 56.0, 64.0];
        LIMITS.iter().position(|l| kn < *l).unwrap_or(12) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip() {
        for p in Param::ALL {
            assert_eq!(Param::from_id(p.id()), Some(p));
        }
        assert_eq!(Param::from_id("nope"), None);
    }

    #[test]
    fn grib_codes_map_to_params() {
        assert_eq!(Param::from_grib(0, 2, 2, Some((103, 10.0))), Some(Param::WindU));
        assert_eq!(Param::from_grib(0, 2, 3, Some((103, 10.0))), Some(Param::WindV));
        // Wind at 100 m is not 10 m wind.
        assert_eq!(Param::from_grib(0, 2, 2, Some((103, 100.0))), None);
        assert_eq!(Param::from_grib(0, 2, 22, Some((1, 0.0))), Some(Param::Gust));
        assert_eq!(Param::from_grib(0, 3, 1, Some((101, 0.0))), Some(Param::Pressure));
        assert_eq!(Param::from_grib(0, 3, 0, Some((101, 0.0))), Some(Param::Pressure), "ECMWF msl");
        assert_eq!(Param::from_grib(0, 3, 0, Some((1, 0.0))), None, "surface pressure is not MSLP");
        assert_eq!(Param::from_grib(0, 0, 0, Some((103, 2.0))), Some(Param::Temperature));
        assert_eq!(Param::from_grib(10, 0, 3, Some((1, 0.0))), Some(Param::WaveHeight));
        assert_eq!(Param::from_grib(10, 0, 10, None), Some(Param::WaveDir));
        assert_eq!(Param::from_grib(10, 0, 15, None), Some(Param::WavePeriod));
        assert_eq!(Param::from_grib(10, 1, 2, None), Some(Param::CurrentU));
        assert_eq!(Param::from_grib(0, 9, 9, None), None);
    }

    #[test]
    fn the_codes_written_read_back_as_the_same_param() {
        for p in Param::ALL {
            let (d, c, n, st, sv) = p.grib_codes();
            assert_eq!(Param::from_grib(d, c, n, Some((st, sv as f64))), Some(p), "{p:?}");
        }
    }

    #[test]
    fn every_group_param_is_a_known_param() {
        for g in Group::ALL {
            assert!(!g.params().is_empty());
            for p in g.params() {
                assert!(Param::ALL.contains(p));
            }
        }
    }

    #[test]
    fn conversions_and_beaufort() {
        assert!((units::ms_to_kn(5.14444) - 10.0).abs() < 1e-3);
        assert_eq!(units::pa_to_hpa(101_325.0), 1013.25);
        assert!((units::k_to_c(273.15)).abs() < 1e-4);
        assert_eq!(units::beaufort(0.0), 0);
        assert_eq!(units::beaufort(12.0), 4);
        assert_eq!(units::beaufort(30.0), 7);
        assert_eq!(units::beaufort(80.0), 12);
    }
}

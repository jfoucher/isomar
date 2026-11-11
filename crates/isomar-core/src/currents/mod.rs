//! Surface currents for the weather router and the chart: their own source, beside the wind models and
//! shared by all of them (`design/currents.md`, `design/currents-plan.md`).
//!
//! Gridded current forecasts (Copernicus, an imported GRIB) are kept in a `WeatherStore` per source, as
//! `Param::CurrentU/V` (the total current) and, for SMOC, `Param::ResidualU/V` (without the tide). Tidal
//! atlases are phased by the tide at their reference port. `field::CurrentField` composes them all,
//! finest first, and `WithCurrents` gives a wind model's environment that one current.

pub mod atlas;
pub mod blosc;
pub mod coefficient;
pub mod copernicus;
pub mod field;
pub mod synthetic;
pub mod zarr;

use crate::geo::LatLon;
use crate::routing::isochrone::Environment;
use chrono::{DateTime, Utc};

pub use field::CurrentField;

/// Where a current comes from, in the order the field prefers them: an imported GRIB first (someone
/// who imports one means it), then the finest grids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CurrentSource {
    /// Currents in a GRIB file the user opened.
    Imported,
    /// A Shom tidal-stream atlas (the tide alone, phased by the reference port's high water), plus
    /// SMOC's residual where it is loaded.
    Atlas,
    /// Copernicus North-West Shelf (AMM15, 1.5 km, tidal).
    NwShelf,
    /// Copernicus Iberia–Biscay–Ireland (1/36°, tidal).
    Ibi,
    /// Copernicus Mediterranean (4.2 km).
    Med,
    /// Copernicus Baltic (~2 km).
    Baltic,
    /// Copernicus global Surface Merged Ocean Currents (1/12°): circulation plus a tide from FES2014.
    Smoc,
    /// A made-up current, for the demo (`ISOMAR_DEMO_CURRENTS`) and tests.
    Demo,
}

impl CurrentSource {
    pub const ALL: [CurrentSource; 8] = [CurrentSource::Imported, CurrentSource::Atlas, CurrentSource::NwShelf, CurrentSource::Ibi, CurrentSource::Med, CurrentSource::Baltic, CurrentSource::Smoc, CurrentSource::Demo];

    /// The Copernicus models, finest first.
    pub const COPERNICUS: [CurrentSource; 5] = [CurrentSource::NwShelf, CurrentSource::Ibi, CurrentSource::Med, CurrentSource::Baltic, CurrentSource::Smoc];

    pub fn id(self) -> &'static str {
        match self {
            CurrentSource::Imported => "imported",
            CurrentSource::Atlas => "atlas",
            CurrentSource::NwShelf => "nwshelf",
            CurrentSource::Ibi => "ibi",
            CurrentSource::Med => "med",
            CurrentSource::Baltic => "baltic",
            CurrentSource::Smoc => "smoc",
            CurrentSource::Demo => "demo",
        }
    }

    pub fn from_id(s: &str) -> Option<CurrentSource> {
        CurrentSource::ALL.into_iter().find(|c| c.id() == s)
    }

    /// The name shown, a proper name (not translated).
    pub fn label(self) -> &'static str {
        match self {
            CurrentSource::Imported => "GRIB",
            CurrentSource::Atlas => "Shom",
            CurrentSource::NwShelf => "Copernicus NW Shelf",
            CurrentSource::Ibi => "Copernicus IBI",
            CurrentSource::Med => "Copernicus Med",
            CurrentSource::Baltic => "Copernicus Baltic",
            CurrentSource::Smoc => "Copernicus SMOC",
            CurrentSource::Demo => "Demo",
        }
    }

    /// Its grid, in words: "1.5 km".
    pub fn resolution(self) -> &'static str {
        match self {
            CurrentSource::Imported | CurrentSource::Demo => "",
            CurrentSource::Atlas => "2 km–250 m",
            CurrentSource::NwShelf => "1.5 km",
            CurrentSource::Ibi => "3 km",
            CurrentSource::Med => "4 km",
            CurrentSource::Baltic => "2 km",
            CurrentSource::Smoc => "9 km",
        }
    }

    /// What the chart must credit while it shows this source's current (the licences ask for it).
    pub fn credit(self) -> Option<&'static str> {
        match self {
            CurrentSource::Atlas => Some("Shom"),
            CurrentSource::NwShelf | CurrentSource::Ibi | CurrentSource::Med | CurrentSource::Baltic | CurrentSource::Smoc => Some("E.U. Copernicus Marine Service Information"),
            CurrentSource::Imported | CurrentSource::Demo => None,
        }
    }

    /// Whether its current turns with the tide.
    pub fn has_tide(self) -> bool {
        !matches!(self, CurrentSource::Med | CurrentSource::Baltic)
    }

    /// Whether it resolves a tidal race well enough to correct a boat's speed by (`perf`): the shelf
    /// models and the atlases, not the 9 km global grid that smooths races away.
    pub fn resolves_races(self) -> bool {
        matches!(self, CurrentSource::Atlas | CurrentSource::NwShelf | CurrentSource::Ibi | CurrentSource::Imported | CurrentSource::Demo)
    }
}

/// A wind model's wind, waves and gusts, and the current of `currents`: every wind model sails on the
/// same current. The wind model's own current, if it had one, is not used, so it is never counted twice.
pub struct WithCurrents<'a, W: Environment> {
    pub wind: W,
    pub currents: &'a CurrentField,
}

impl<W: Environment> Environment for WithCurrents<'_, W> {
    fn wind(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.wind.wind(p, t)
    }

    fn has_wind(&self, p: &LatLon, t: DateTime<Utc>) -> bool {
        self.wind.has_wind(p, t)
    }

    fn current(&self, p: &LatLon, t: DateTime<Utc>) -> Option<(f64, f64)> {
        self.currents.sample(p, t)
    }

    fn has_currents(&self) -> bool {
        !self.currents.is_empty()
    }

    fn wave_height(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.wind.wave_height(p, t)
    }

    fn gust(&self, p: &LatLon, t: DateTime<Utc>) -> Option<f64> {
        self.wind.gust(p, t)
    }

    fn horizon(&self) -> Option<DateTime<Utc>> {
        self.wind.horizon()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_and_copernicus_is_credited() {
        for s in CurrentSource::ALL {
            assert_eq!(CurrentSource::from_id(s.id()), Some(s));
        }
        for s in CurrentSource::COPERNICUS {
            assert!(s.credit().unwrap().contains("Copernicus"));
        }
        assert_eq!(CurrentSource::Atlas.credit(), Some("Shom"));
    }
}

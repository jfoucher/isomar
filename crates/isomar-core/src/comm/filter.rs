//! Per-connection sentence filters, so a noisy instrument can be silenced
//! without unplugging it.

use super::nmea::parse_sentence;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterMode {
    #[default]
    Off,
    /// Only the listed sentences pass.
    Allow,
    /// Everything but the listed sentences passes.
    Deny,
}

/// A list of sentence types (`RMC`) or talker+type (`GPRMC`), compared
/// case-insensitively.
#[derive(Clone, Debug, Default)]
pub struct LineFilter {
    pub mode: FilterMode,
    pub items: Vec<String>,
}

impl LineFilter {
    pub fn new(mode: FilterMode, items: &[&str]) -> LineFilter {
        LineFilter { mode, items: items.iter().map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).collect() }
    }

    /// Parses `"RMC, GPGGA"`.
    pub fn from_text(mode: FilterMode, text: &str) -> LineFilter {
        let items: Vec<&str> = text.split([',', ' ', ';']).collect();
        LineFilter::new(mode, &items)
    }

    pub fn accepts(&self, line: &str) -> bool {
        if self.mode == FilterMode::Off {
            return true;
        }
        let listed = match parse_sentence(line) {
            Ok(s) => self.items.iter().any(|i| *i == s.kind || *i == format!("{}{}", s.talker, s.kind)),
            // Lines that aren't sentences (or fail the checksum) are never "listed".
            Err(_) => false,
        };
        match self.mode {
            FilterMode::Allow => listed,
            FilterMode::Deny => !listed,
            FilterMode::Off => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comm::nmea::frame;

    #[test]
    fn modes() {
        let rmc = frame("GPRMC,1,A", '$');
        let gga = frame("GPGGA,1", '$');
        let allow = LineFilter::new(FilterMode::Allow, &["RMC"]);
        assert!(allow.accepts(&rmc) && !allow.accepts(&gga));
        let deny = LineFilter::new(FilterMode::Deny, &["rmc"]);
        assert!(!deny.accepts(&rmc) && deny.accepts(&gga));
        assert!(LineFilter::default().accepts(&rmc));
    }

    #[test]
    fn talker_specific_entries() {
        let gp = frame("GPRMC,1,A", '$');
        let gn = frame("GNRMC,1,A", '$');
        let f = LineFilter::from_text(FilterMode::Deny, "GPRMC");
        assert!(!f.accepts(&gp) && f.accepts(&gn));
    }

    #[test]
    fn text_parsing_tolerates_separators() {
        let f = LineFilter::from_text(FilterMode::Allow, " rmc, gga ;vtg  ");
        assert_eq!(f.items, ["RMC", "GGA", "VTG"]);
    }

    #[test]
    fn garbage_is_dropped_by_allow_and_kept_by_deny() {
        assert!(!LineFilter::new(FilterMode::Allow, &["RMC"]).accepts("garbage"));
        assert!(LineFilter::new(FilterMode::Deny, &["RMC"]).accepts("garbage"));
    }
}

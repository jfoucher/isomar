//! Text for the chart that both shells show the same way.

use crate::core::geo::{self, LatLon};

/// A position as a chart reads it: `48°23.400′N 004°29.400′W`.
pub fn position_text(p: &LatLon) -> String {
    format!("{} {}", geo::format_lat(p.lat), geo::format_lon(p.lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_read_like_a_chart() {
        assert_eq!(position_text(&LatLon::new(48.39, -4.49)), "48°23.400′N 004°29.400′W");
    }
}

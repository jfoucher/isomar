//! Lettering along a path, as on a paper chart: a feature's centreline (`spine`), found on a grid of
//! cells over it (`mask`), and a name set along it with the room it needs (`fit`). Everything is in
//! Web Mercator "units": the world at zoom 0 is 256 units across and y grows southwards, like the
//! screen, so at zoom `z` one unit is `geo::world_size(z) / 256` pixels.

pub mod coast;
pub mod fit;
pub mod mask;
pub mod spine;

pub use fit::{FitParams, Fitted, GlyphAt, LINE_SPACING, Line, box_of, fit, fit_avoiding, fit_lines_avoiding, fit_words};
pub use mask::Mask;
pub use spine::{Spine, spine, spine_in};

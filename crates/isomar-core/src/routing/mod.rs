//! Routing: boat polars, a land mask, the isochrone (weather) router, and the
//! automatic router that keeps clear of land and shallows, and the weather
//! window that finds when to leave.

pub mod alternatives;
pub mod along;
pub mod auto;
pub mod compare;
pub mod ensemble;
pub mod isochrone;
pub mod land;
pub mod library;
pub mod navmask;
pub mod nested;
pub mod passage;
pub mod plan;
pub mod polar;
pub mod seaway;
pub mod stats;
pub mod window;

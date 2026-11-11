//! Weather data: what the fields are, where they come from, how to fetch
//! just the part we need, and how to read them.

pub mod cache;
pub mod download;
pub mod grib;
pub mod gribwrite;
pub mod model;
pub mod planner;
pub mod source;
pub mod store;

#[cfg(test)]
pub(crate) mod testgrib;

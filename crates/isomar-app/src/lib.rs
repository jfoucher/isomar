//! The application engine behind Isomar's windows: it knows nothing of GTK,
//! so a different shell (macOS) can drive it.

#![allow(clippy::type_complexity, clippy::too_many_arguments)]

// Keep the domain code reachable by its old paths.
pub use isomar_core as core;
pub use isomar_core::{config, i18n};

pub mod actions;
pub mod aidinfo;
pub mod api;
pub mod app;
pub mod chips;
pub mod context;
pub mod currents;
pub mod data;
pub mod debug;
pub mod dem;
pub mod emodnet;
pub mod engine;
pub mod featureinfo;
pub mod flows;
pub mod fonts;
pub mod hit;
pub mod host;
pub mod jobs;
pub mod key;
pub mod layers;
pub mod palette;
pub mod places;
pub mod polars;
pub mod probe;
pub mod render;
pub mod pump;
pub mod platform;
pub mod router;
pub mod shell;
pub mod symbols;
pub mod text;
pub mod tokens;
pub mod units;
pub mod variation;
pub mod tiles;
pub mod view;
pub mod vm;
pub mod weather;

//! The performance log and what is learnt from it.
//!
//! While the boat is under way, the instruments are averaged over 10 s windows ([`aggregator`]) into
//! [`sample::Sample`]s, kept in the navigation database with the forecast's wind and waves at each. From
//! them, off the main thread ([`analyse`]):
//!
//! - which samples are steady sailing and what each is worth ([`filter`]: tier A with wind instruments
//!   and a speedo, C with wind instruments over the ground, D with a model's wind only);
//! - what the instruments get wrong, found from the data itself ([`calibrate`]: the masthead unit's
//!   offset, the angle error that shows as a wind shift at every tack, the speedo's factor);
//! - a polar learnt from tiers A and C ([`learn`]), blended with the boat's polar where data is thin;
//! - how fast the boat sails against its polar, from any tier ([`factors`]). A model's wind is too far off
//!   (±25 degrees, ±30%) to learn the polar's shape from: binned, it smears the polar along the angle and
//!   the router then points too high. So tier D only ever gives these factors, fitted against the polar
//!   blurred by the same errors;
//! - how good the forecast was against the instruments ([`forecast_check`]), how waves slow the boat
//!   ([`waves`]), and planned against sailed passages ([`passage`]).
//!
//! Whether the boat is sailing is the user's switch, except that an engine reporting its revolutions
//! sets it ([`engine`]); without engine data a heuristic only suggests motoring ([`hint`]).

pub mod aggregator;
pub mod analyse;
pub mod calibrate;
pub mod current_check;
pub mod engine;
pub mod factors;
pub mod filter;
pub mod forecast_check;
pub mod hint;
pub mod learn;
pub mod log;
pub mod model_check;
pub mod passage;
pub mod rng;
pub mod sample;
pub mod stats;
pub mod synth;
pub mod waves;

pub use log::PerfLog;
pub use sample::{Hint, ModeSource, ModelWind, SailMode, Sample};

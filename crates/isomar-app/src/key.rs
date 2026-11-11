//! Keys the chart reacts to, in no toolkit's terms. A shell maps its own key
//! codes to these and passes anything else as `Other`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    ZoomIn,
    ZoomOut,
    Left,
    Right,
    Up,
    Down,
    Escape,
    Enter,
    Backspace,
    Other,
}

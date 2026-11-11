// Reading token values from `tokens.json`. Included with `include!` by the generators
// (`isomar-app/build.rs` for the renderer, `isomar-gtk/build.rs` for the GNOME stylesheets,
// `xtask` for Swift), so every one parses a value the same way.

/// `#rrggbb`, `#rgb`, `rgb(r,g,b)` or `rgba(r,g,b,a)` as `(r, g, b, a)` in 0..1.
#[allow(dead_code)]
fn colour(s: &str) -> (f64, f64, f64, f64) {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let h = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect::<String>() } else { h.to_string() };
        let n = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
        return (n(0), n(2), n(4), 1.0);
    }
    let inner = s.split('(').nth(1).and_then(|r| r.strip_suffix(')')).unwrap_or_else(|| panic!("colour {s}"));
    let p: Vec<f64> = inner.split(',').map(|x| x.trim().parse().unwrap()).collect();
    (p[0] / 255.0, p[1] / 255.0, p[2] / 255.0, p.get(3).copied().unwrap_or(1.0))
}

/// A number from `34px`, `25mm`, `500ms`, `4s`, in milliseconds for times and the unit's own
/// scale otherwise.
#[allow(dead_code)]
fn number_ms(s: &str) -> f64 {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("ms") {
        return v.parse().unwrap();
    }
    if let Some(v) = s.strip_suffix("px").or_else(|| s.strip_suffix("mm")) {
        return v.parse().unwrap();
    }
    if let Some(v) = s.strip_suffix('s') {
        return v.parse::<f64>().unwrap() * 1000.0;
    }
    s.parse().unwrap_or_else(|_| panic!("number {s}"))
}

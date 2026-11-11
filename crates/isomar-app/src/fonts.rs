//! Every cairo font face the chart uses, kept alive for good.
//!
//! cairo 1.18 keeps the faces of one font file (one per set of font options) in a list that it walks,
//! adds to and unlinks from with no lock, although every thread's Pango shares it: the tile threads,
//! the main thread measuring names, and GTK's labels all use Adwaita Sans. A face freed on one thread
//! while another walked or added to the list left a freed face in it, and GTK crashed drawing a label
//! (`_cairo_ft_font_face_create`). A face that is never freed is never unlinked, so each face is
//! held here once it has been seen; and the usual ones are made on the main thread before any tile
//! thread starts (`warm_up`), so the tile threads only find them in the list and never add to it.

use pangocairo::prelude::*;
use std::sync::{Mutex, Once};

/// The faces held, by address: a held face is never freed, so its address is never another's.
static HELD: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// Keeps `face` alive for as long as the process runs.
pub fn hold(face: &cairo::FontFace) {
    let key = face.to_raw_none() as usize;
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    if !held.contains(&key) {
        held.push(key);
        std::mem::forget(face.clone());
    }
}

/// Keeps alive the face of every font `layout` uses, the fallbacks for other scripts too.
pub fn hold_layout(layout: &pango::Layout) {
    let mut iter = layout.iter();
    loop {
        if let Some(run) = iter.run_readonly()
            && let Some(font) = run.item().analysis().font().dynamic_cast_ref::<pangocairo::Font>()
            && let Some(scaled) = font.scaled_font()
        {
            hold(&scaled.font_face());
        }
        if !iter.next_run() {
            break;
        }
    }
}

/// Makes and keeps the faces the chart's text uses, in each style and with each set of font options
/// it is drawn with. Called on the main thread before the tile threads start; once.
pub fn warm_up() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(surface) = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1) else { return };
        for options in [None, Some(crate::layers::names::path_font_options())] {
            let Ok(cr) = cairo::Context::new(&surface) else { return };
            if let Some(o) = &options {
                cr.set_font_options(o);
            }
            // Cairo's own text: a toy face holds the face it resolved to.
            for weight in [cairo::FontWeight::Normal, cairo::FontWeight::Bold] {
                cr.select_font_face(crate::layers::FONT_FAMILY, cairo::FontSlant::Normal, weight);
                let _ = cr.text_extents("0");
                hold(&cr.font_face());
            }
            // Pango's, in every weight and slant the names take.
            let layout = pangocairo::functions::create_layout(&cr);
            let mut font = pango::FontDescription::new();
            font.set_family(crate::layers::FONT_FAMILY);
            font.set_absolute_size(12.0 * pango::SCALE as f64);
            layout.set_font_description(Some(&font));
            layout.set_markup("Aa <b>Aa</b> <i>Aa</i> <b><i>Aa</i></b>");
            hold_layout(&layout);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_faces_outlive_their_layout() {
        warm_up();
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).unwrap();
        let cr = cairo::Context::new(&surface).unwrap();
        let layout = pangocairo::functions::create_layout(&cr);
        layout.set_markup("Groix <i>Glénan</i>");
        hold_layout(&layout);
        let mut faces = Vec::new();
        let mut iter = layout.iter();
        loop {
            if let Some(run) = iter.run_readonly()
                && let Some(font) = run.item().analysis().font().dynamic_cast_ref::<pangocairo::Font>()
                && let Some(scaled) = font.scaled_font()
            {
                faces.push(scaled.font_face());
            }
            if !iter.next_run() {
                break;
            }
        }
        assert!(!faces.is_empty());
        let held = HELD.lock().unwrap();
        for face in &faces {
            assert!(held.contains(&(face.to_raw_none() as usize)));
            // Ours, the one held, and the layout's font's.
            assert!(face.reference_count() >= 2);
        }
    }
}

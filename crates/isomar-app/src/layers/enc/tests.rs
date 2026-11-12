use super::*;
use crate::view::View;

/// The Iroise cell of the test data, if it is there.
fn iroise() -> Option<Cell> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/shom-iroise-s57/S57_OPENDATA_IROISEE/FR571230/FR571230.000");
    p.exists().then(|| load_cell(&p).unwrap())
}

#[test]
fn a_click_on_a_real_wreck_tells_what_it_is_and_one_on_open_water_tells_nothing() {
    let Some(cell) = iroise() else { return };
    let layer = EncLayer::new(Rc::new(|| {}));
    layer.add_cell(cell.clone());
    let wreck = cell.features.iter().find(|f| f.objl == obj::WRECKS && f.attr(attr::VALSOU).is_some()).unwrap();
    let Geom::Point(p) = wreck.geom else { panic!() };
    let vi = ViewInfo { view: View { center: LatLon::new(p[1], p[0]), zoom: 15.0 }, w: 800.0, h: 600.0 };
    let d = layer.features_at(&vi, 400.0, 300.0, 10.0).into_iter().next().expect("the wreck under the pointer").description;
    assert!(d.title.contains("Wreck") || d.subtitle.as_deref() == Some("Wreck"), "{d:?}");
    assert!(d.rows.iter().any(|r| r.0 == "Value of sounding"), "{:?}", d.rows);
    // A click well off it finds that or something else, but never the wreck.
    let far = layer.features_at(&vi, 40.0, 40.0, 10.0).into_iter().next();
    assert!(far.is_none_or(|f| f.description.rows != d.rows));
}

#[test]
fn a_coarse_cell_under_a_finer_one_is_not_reported_where_the_finer_one_has_data() {
    let square = |d: f64| vec![vec![[-d, -d], [d, -d], [d, d], [-d, d], [-d, -d]]];
    let mk = |cscl: u32, d: f64, features: Vec<Feature>| Cell {
        name: format!("C{cscl}"),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl,
        features,
        bounds: Bounds { min_lon: -d, min_lat: -d, max_lon: d, max_lat: d },
        warnings: vec![],
        skipped_updates: vec![],
    };
    let caution = Feature { rcid: 1, objl: obj::CTNARE, attrs: Default::default(), geom: Geom::Area(square(0.5)) };
    let layer = EncLayer::new(Rc::new(|| {}));
    layer.add_cell(mk(90_000, 0.5, vec![caution]));
    layer.add_cell(mk(22_000, 0.05, vec![]));
    let at = |lon: f64| ViewInfo { view: View { center: LatLon::new(0.0, lon), zoom: 14.0 }, w: 800.0, h: 600.0 };
    assert!(layer.features_at(&at(0.0), 400.0, 300.0, 10.0).is_empty(), "the finer cell is drawn there");
    assert_eq!(layer.features_at(&at(0.2), 400.0, 300.0, 10.0).len(), 1, "past it the coarse cell is");
}

#[test]
fn what_a_cell_gives_a_scamin_is_hidden_from_smaller_scales_except_the_basics() {
    let f = |objl, scamin: Option<&str>| Feature {
        rcid: 1,
        objl,
        attrs: scamin.map(|s| (attr::SCAMIN, s.to_string())).into_iter().collect(),
        geom: Geom::Point([0.0, 0.0]),
    };
    assert!(scamin_shows(&f(obj::DEPCNT, Some("22000")), 22_000.0), "at its scale");
    assert!(!scamin_shows(&f(obj::DEPCNT, Some("22000")), 90_000.0), "a contour its author hid from 1:90 000");
    assert!(scamin_shows(&f(obj::DEPCNT, None), 1e7), "no SCAMIN, always");
    assert!(scamin_shows(&f(obj::DEPARE, Some("22000")), 90_000.0), "depth areas always show");
    assert!(scamin_shows(&f(obj::LIGHTS, Some("22000")), 90_000.0), "marks and lights have their own rules");
    assert!(!scamin_shows(&f(obj::UWTROC, Some("22000")), 90_000.0));
    assert!(scamin_shows(&f(obj::SOUNDG, Some("22000")), 20_000.0), "sounding at scale");
    assert!(!scamin_shows(&f(obj::SOUNDG, Some("22000")), 90_000.0), "sounding with SCAMIN hidden at smaller scale");
    assert!(scamin_shows(&f(obj::SOUNDG, None), 90_000.0), "sounding without SCAMIN always shows in scamin_shows");
}

#[test]
fn a_real_cell_gives_its_magnetic_variation() {
    let Some(cell) = iroise() else { return };
    let layer = EncLayer::new(Rc::new(|| {}));
    let b = cell.summary().bounds;
    let (lon, lat) = ((b.min_lon + b.max_lon) / 2.0, (b.min_lat + b.max_lat) / 2.0);
    layer.add_cell(cell);
    let v = layer.variation_at(lon, lat).expect("the cell has a MAGVAR");
    assert!((-10.0..10.0).contains(&v.value), "{v:?}");
    assert!(layer.variation_at(lon + 20.0, lat).is_none(), "far outside its cells");
}

#[test]
fn a_real_cell_gives_its_anchorages() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata");
    let p = root.join("noaa-us5ma20m/ENC_ROOT/US5MA20M/US5MA20M.000");
    if !p.exists() {
        return;
    }
    let cell = load_cell(&p).unwrap();
    let found: Vec<Anchorage> = cell.features.iter().filter_map(anchorage_of).collect();
    for a in &found {
        eprintln!("{:?} {:?} {:?} {} rings at {:?}", a.kind, a.name, a.categories, a.rings.len(), a.pos);
    }
    let raw = cell.features.iter().filter(|f| matches!(f.objl, obj::ACHARE | obj::ACHBRT)).count();
    assert_eq!(found.iter().filter(|a| matches!(a.kind, AnchorageKind::Anchorage | AnchorageKind::Berth)).count(), raw);
    assert!(found.iter().all(|a| a.rings.is_empty() || a.contains([a.pos.lon, a.pos.lat])), "each anchor inside its area");
}

#[test]
fn an_enc_anchorage_follows_its_scamin_else_the_zooms_of_the_others() {
    let feature = |attrs: &[(u16, &str)]| Feature {
        rcid: 1,
        objl: obj::ACHARE,
        attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect(),
        geom: Geom::Area(vec![vec![[0.0, 0.0], [0.01, 0.0], [0.01, 0.01], [0.0, 0.01]]]),
    };
    let with = feature(&[(attr::SCAMIN, "89999")]);
    let a = anchorage_of(&with).unwrap();
    // Far out, but the cell asks for it down to 1:89 999.
    assert!(anchorage_shown(&with, &a, 8.0, 80_000.0));
    assert!(!anchorage_shown(&with, &a, 14.0, 90_000.0), "past its SCAMIN, however close the zoom");
    let without = feature(&[]);
    assert!(!anchorage_shown(&without, &a, 9.0, 80_000.0));
    assert!(anchorage_shown(&without, &a, 10.0, 800_000.0), "an area from zoom 10");
}

fn small_view() -> ViewInfo {
    ViewInfo { view: View { center: LatLon::new(37.8, -122.4), zoom: 12.0 }, w: 200.0, h: 150.0 }
}

/// A ring of `n` points round screen point (`cx`, `cy`), `r` pixels across, as lon/lat.
fn ring(vi: &ViewInfo, cx: f64, cy: f64, r: f64, n: usize) -> Vec<[f64; 2]> {
    (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let p = vi.position_at(cx + r * a.cos(), cy + r * a.sin());
            [p.lon, p.lat]
        })
        .collect()
}

fn path_len(cr: &cairo::Context) -> usize {
    cr.copy_path().map(|p| p.iter().count()).unwrap_or(0)
}

/// Fills and strokes `ring` traced with `f` and returns the pixels.
fn render(vi: &ViewInfo, f: impl Fn(&cairo::Context)) -> Vec<u8> {
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, vi.w as i32, vi.h as i32).unwrap();
    {
        let cr = cairo::Context::new(&surface).unwrap();
        f(&cr);
        cr.set_source_rgb(0.2, 0.4, 0.8);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.set_line_width(2.5);
        let _ = cr.stroke();
    }
    surface.data().unwrap().to_vec()
}

#[test]
fn a_cell_tells_its_index_contours_from_the_spacing_of_its_lines() {
    let line = |e: &str| Feature { rcid: 1, objl: obj::LNDELV, attrs: [(attr::ELEVAT, e.to_string())].into_iter().collect(), geom: Geom::Line(vec![[0.0, 0.0], [1.0, 1.0]]) };
    let spot = Feature { rcid: 2, objl: obj::LNDELV, attrs: [(attr::ELEVAT, "7".to_string())].into_iter().collect(), geom: Geom::Point([0.0, 0.0]) };
    let e = Elev::of(&["10", "20", "30", "40", "50", "60", "20"].iter().map(|e| line(e)).chain([spot]).collect::<Vec<_>>());
    assert!(e.lines);
    assert_eq!(e.interval, 10.0);
    assert!(e.is_index(50.0) && e.is_index(100.0) && e.is_index(0.0));
    assert!(!e.is_index(10.0) && !e.is_index(40.0) && !e.is_index(60.0));
    // Spot heights alone are not contour lines: the downloaded relief supplies those.
    let spots = Elev::of(&[Feature { rcid: 3, objl: obj::LNDELV, attrs: [(attr::ELEVAT, "7".to_string())].into_iter().collect(), geom: Geom::Point([0.0, 0.0]) }]);
    assert_eq!(spots, Elev::default());
    assert!(!spots.is_index(50.0));
}

#[test]
fn a_cell_with_no_land_contours_leaves_them_to_the_downloaded_relief() {
    // The tests run in the crate's folder; the data is at the workspace root.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata");
    let noaa = root.join("noaa-us5ma20m/ENC_ROOT/US5MA20M/US5MA20M.000");
    let shom = root.join("shom-iroise-s57/S57_OPENDATA_IROISEE/FR571220/FR571220.000");
    if !noaa.exists() || !shom.exists() {
        return;
    }
    let noaa = load_cell(&noaa).unwrap();
    assert!(!Elev::of(&noaa.features).lines, "NOAA's cell has no land contour lines");
    let e = Elev::of(&load_cell(&shom).unwrap().features);
    assert!(e.lines && e.interval > 0.0, "{e:?}");
}

#[test]
fn a_depth_contour_takes_its_value_from_valdco() {
    let contour = |attrs: &[(u16, &str)]| Feature {
        rcid: 1,
        objl: obj::DEPCNT,
        attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect(),
        geom: Geom::Line(vec![]),
    };
    assert_eq!(contour(&[(attr::VALDCO, "10")]).num(attr::VALDCO), Some(10.0));
    assert_eq!(contour_value(&contour(&[(attr::VALDCO, "10")])), 10.0);
    assert_eq!(contour_value(&contour(&[(attr::VALDCO, "0")])), 0.0);
    assert_eq!(contour_value(&contour(&[(attr::DRVAL1, "5")])), 5.0);
    assert_eq!(contour_value(&contour(&[])), 0.0);
}

#[test]
fn tracing_leaves_out_what_is_off_screen_and_draws_the_same() {
    let vi = small_view();
    // A ring much bigger than the view that crosses it, with vertices about 0.6 px apart.
    let big = ring(&vi, 100.0, 75.0, 400.0, 4000);
    let plain = render(&vi, |cr| {
        for (i, p) in big.iter().enumerate() {
            let (x, y) = vi.to_screen(&LatLon::new(p[1], p[0]));
            if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
        }
        cr.close_path();
    });
    let traced = render(&vi, |cr| trace(&vi, cr, &big, true));
    let worst = plain.iter().zip(&traced).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 2, "the traced ring looks different: {worst}");
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    trace(&vi, &cr, &big, true);
    assert!(path_len(&cr) < 1000, "{} path elements for 4000 points", path_len(&cr));
}

#[test]
fn a_ring_off_screen_or_smaller_than_a_pixel_is_a_few_points() {
    let vi = small_view();
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 1, 1).unwrap();
    let cr = cairo::Context::new(&surface).unwrap();
    trace(&vi, &cr, &ring(&vi, -500.0, 75.0, 100.0, 5000), true);
    assert!(path_len(&cr) <= 8, "off to the left: {}", path_len(&cr));
    cr.new_path();
    trace(&vi, &cr, &ring(&vi, 100.0, 75.0, 0.2, 500), true);
    assert!(path_len(&cr) <= 4, "a speck: {}", path_len(&cr));
}

#[test]
fn scale_follows_zoom() {
    let a = view_scale(48.0, 10.0);
    let b = view_scale(48.0, 12.0);
    assert!((a / b - 4.0).abs() < 0.01);
    // About 1:25 000 at zoom 13.5 at this latitude, a harbour approach view.
    assert!((10_000.0..60_000.0).contains(&view_scale(48.0, 13.5)), "{}", view_scale(48.0, 13.5));
}

#[test]
fn harbour_charts_do_not_show_when_zoomed_far_out() {
    assert!(cell_visible(25_000, 20_000.0));
    assert!(cell_visible(25_000, 70_000.0));
    assert!(!cell_visible(25_000, 500_000.0));
    assert!(cell_visible(0, 1.0e9), "no scale means always");
    assert!(cell_visible(1_500_000, 3_000_000.0), "overview cells span far");
}

#[test]
fn cell_summary_visible_applies_scale_cutoffs_and_max_band() {
    let dummy_cell = |name: &str, cscl: u32| CellSummary {
        name: name.into(),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl,
        bounds: Bounds::EMPTY,
        features: 0,
        warnings: 0,
        cover: Default::default(),
    };
    let harbour = dummy_cell("FR571220.000", 10_000); // Band 5
    let approach = dummy_cell("FR401010.000", 45_000); // Band 4
    let coastal = dummy_cell("FR301010.000", 150_000); // Band 3

    // Band 5 Harbour chart shows at harbor scales, but not when zoomed out to coastal scale (> 55,000)
    assert!(cell_summary_visible(&harbour, 20_000.0, None));
    assert!(!cell_summary_visible(&harbour, 60_000.0, None));

    // Band 4 Approach chart shows at approach scales, but not at overview (> 220,000)
    assert!(cell_summary_visible(&approach, 80_000.0, None));
    assert!(!cell_summary_visible(&approach, 250_000.0, None));

    // When max_band is limited to Band 3 (Coastal), Band 4 and Band 5 are hidden even at large scales
    assert!(!cell_summary_visible(&harbour, 10_000.0, Some(3)));
    assert!(!cell_summary_visible(&approach, 45_000.0, Some(3)));
    assert!(cell_summary_visible(&coastal, 150_000.0, Some(3)));

    // When max_band is limited to Band 4 (Approach), Band 5 is hidden but Band 4 is shown
    assert!(!cell_summary_visible(&harbour, 10_000.0, Some(4)));
    assert!(cell_summary_visible(&approach, 45_000.0, Some(4)));
}

#[test]
fn depth_areas_share_the_grid_tints() {
    let t = |d1, d2| depth_area_colour(d1, d2, 10.0);
    assert_eq!(t(0.0, Some(3.0)), Token::ChartShallow);
    assert_eq!(t(5.0, Some(9.0)), Token::ChartMedium);
    assert_eq!(t(50.0, Some(100.0)), Token::ChartDeep);
    assert_eq!(t(20.0, None), depth_color(-20.0, 10.0), "no upper limit: the shallow one");
    assert_eq!(t(-1.0, Some(0.0)), Token::ChartDrying);
}

#[test]
fn soundings_print_like_a_chart() {
    assert_eq!(sounding_parts(4.26), ("4".to_string(), Some('3'), false));
    assert_eq!(sounding_parts(12.0), ("12".to_string(), Some('0'), false));
    assert_eq!(sounding_parts(35.7), ("35".to_string(), None, false));
    assert_eq!(sounding_parts(120.9), ("120".to_string(), None, false));
}

#[test]
fn drying_heights_lose_their_sign_and_are_underlined() {
    assert_eq!(sounding_parts(-1.3), ("1".to_string(), Some('3'), true));
    assert_eq!(sounding_parts(0.0), ("0".to_string(), Some('0'), false));
}

#[test]
fn soundings_thin_out_where_the_bottom_is_flat() {
    let cell = (50.0, 30.0);
    // A flat bottom at 20 m, one sounding per cell: few are kept, and none crowd each other.
    let flat: Vec<_> = (0..20).flat_map(|i| (0..20).map(move |j| (20.0 + (i + j) as f64 * 0.01, i as f64 * 50.0, j as f64 * 30.0))).collect();
    let kept = thin_soundings(flat.clone(), cell);
    assert!(kept.len() < flat.len() / 6, "{} of {}", kept.len(), flat.len());
    // A steep slope keeps more, the shallow end most densely.
    let steep: Vec<_> = (0..20).flat_map(|i| (0..20).map(move |j| (2.0 + i as f64 * 6.0, i as f64 * 50.0, j as f64 * 30.0))).collect();
    let on_slope = thin_soundings(steep, cell);
    assert!(on_slope.len() > kept.len() * 4 / 3, "{} vs {}", on_slope.len(), kept.len());
    assert!((0..5).all(|i| on_slope.iter().any(|p| p.1 == i as f64 * 50.0)), "every step of the shallow end is printed");
    // Deep water is thinner than shallow water of the same evenness.
    let deep: Vec<_> = flat.iter().map(|p| (p.0 + 80.0, p.1, p.2)).collect();
    assert!(thin_soundings(deep, cell).len() < kept.len());
}

#[test]
fn thinning_never_hides_a_shoal() {
    // A 3 m shoal in a flat 10 m bottom is kept, and so is a drying height next to it.
    let mut pts: Vec<_> = (0..10).flat_map(|i| (0..10).map(move |j| (10.0, i as f64 * 50.0, j as f64 * 30.0))).collect();
    pts.push((3.0, 251.0, 151.0));
    pts.push((-0.5, 300.0, 151.0));
    let kept = thin_soundings(pts, (50.0, 30.0));
    assert_eq!(kept[0], (-0.5, 300.0, 151.0));
    assert_eq!(kept[1], (3.0, 251.0, 151.0));
}

#[test]
fn soundings_show_wherever_their_cell_does() {
    // An overview cell shows its soundings at its own scale, which a fixed zoom never allowed.
    assert!(soundings_visible(1_500_000, 1_500_000.0, 7.0));
    assert!(soundings_visible(25_000, 70_000.0, 12.0));
    assert!(!soundings_visible(25_000, 500_000.0, 10.0));
    assert!(!soundings_visible(0, 1.0e6, 9.0));
    assert!(soundings_visible(0, 1.0e4, 14.0));
}

#[test]
fn a_cape_named_inland_of_its_point_is_written_from_the_tip_and_a_hill_is_not_a_cape() {
    use std::collections::BTreeMap;
    let f = |objl: u16, attrs: &[(u16, &str)], geom: Geom| Feature { rcid: 0, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom };
    // Land west of 2 E, with a point 0.002 degrees wide running east to 2.01 E at 40 N.
    let land = vec![vec![[1.9, 39.9], [2.0, 39.9], [2.0, 39.999], [2.01, 39.999], [2.01, 40.001], [2.0, 40.001], [2.0, 40.1], [1.9, 40.1], [1.9, 39.9]]];
    let cell = Cell {
        name: "T".into(),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl: 20_000,
        features: vec![
            f(obj::LNDARE, &[], Geom::Area(land)),
            // Named 300 m inland of the end of the point, as a cell may.
            f(obj::LNDRGN, &[(attr::OBJNAM, "Montauk Point")], Geom::Point([2.0065, 40.0])),
            f(obj::LNDRGN, &[(attr::OBJNAM, "Signal Hill")], Geom::Point([1.95, 40.0])),
        ],
        bounds: Bounds::EMPTY,
        warnings: vec![],
        skipped_updates: vec![],
    };
    let names = names_of(&cell);
    let cape = names.iter().find(|n| n.text == "Montauk Point").unwrap();
    let axis = cape.out_from.expect("a cape on the coast");
    let tip = to_units([2.01, 40.0]);
    let m_per_unit = 40_075_016.7 * 40f64.to_radians().cos() / 256.0;
    assert!((axis.tip[0] - tip[0]).abs() * m_per_unit < 40.0 && (axis.tip[1] - tip[1]).abs() * m_per_unit < 60.0, "{:?} {tip:?}", axis.tip);
    assert!(axis.out[0] > 0.9, "points east: {:?}", axis.out);
    assert!(cape.spine.is_none());
    let hill = names.iter().find(|n| n.text == "Signal Hill").unwrap();
    assert!(hill.out_from.is_none());
    assert!(is_cape_name("Pointe de Corsen") && is_cape_name("C. de Palos / Cabo de Palos") && !is_cape_name("Pointers Rock"));
}

#[test]
fn a_cove_is_written_out_from_its_mouth_and_open_water_is_not_a_cove() {
    use std::collections::BTreeMap;
    let f = |objl: u16, attrs: &[(u16, &str)], geom: Geom| Feature { rcid: 0, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom };
    // Land west of 2 E, with a cove about 330 m wide and 430 m deep cut into it at 40 N.
    let land = vec![vec![[1.9, 39.9], [2.0, 39.9], [2.0, 39.9985], [1.995, 39.9985], [1.995, 40.0015], [2.0, 40.0015], [2.0, 40.1], [1.9, 40.1], [1.9, 39.9]]];
    let cell = Cell {
        name: "T".into(),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl: 20_000,
        features: vec![
            f(obj::LNDARE, &[], Geom::Area(land)),
            f(obj::SEAARE, &[(attr::OBJNAM, "Anse de Test")], Geom::Point([1.997, 40.0])),
            // A bay by its category, whatever its name.
            f(obj::SEAARE, &[(attr::OBJNAM, "Le Trou"), (attr::CATSEA, "5")], Geom::Point([1.998, 40.0005])),
            f(obj::SEAARE, &[(attr::OBJNAM, "Baie Ouverte")], Geom::Point([2.05, 40.0])),
            f(obj::SEAARE, &[(attr::OBJNAM, "Le Plateau")], Geom::Point([1.997, 40.0])),
            // A small piece of a big bay in the cove is a piece of the bay, not a cove.
            f(obj::SEAARE, &[(attr::OBJNAM, "Grande Baie"), (attr::CATSEA, "5")], Geom::Point([1.997, 39.9995])),
            f(obj::SEAARE, &[(attr::OBJNAM, "Grande Baie"), (attr::CATSEA, "5")], Geom::Area(vec![vec![[2.01, 39.95], [2.2, 39.95], [2.2, 40.05], [2.01, 40.05], [2.01, 39.95]]])),
        ],
        bounds: Bounds::EMPTY,
        warnings: vec![],
        skipped_updates: vec![],
    };
    let names = names_of(&cell);
    let mouth = to_units([2.0, 40.0]);
    let m_per_unit = 40_075_016.7 * 40f64.to_radians().cos() / 256.0;
    for name in ["Anse de Test", "Le Trou"] {
        let axis = names.iter().find(|n| n.text == name).unwrap().out_from.expect("a cove");
        assert!(axis.out[0] > 0.9, "{name} points east, out of the cove: {:?}", axis.out);
        assert!((axis.tip[0] - mouth[0]).abs() * m_per_unit < 80.0, "{name} from its mouth: {:?} {mouth:?}", axis.tip);
    }
    assert!(names.iter().find(|n| n.text == "Baie Ouverte").unwrap().out_from.is_none(), "open water");
    assert!(names.iter().find(|n| n.text == "Le Plateau").unwrap().out_from.is_none(), "not a bay");
    assert!(names.iter().filter(|n| n.text == "Grande Baie").all(|n| n.out_from.is_none()), "a piece of a bigger bay");
}

#[test]
fn names_are_taken_from_waters_capes_and_towns_and_shown_as_the_cell_says() {
    use std::collections::BTreeMap;
    let f = |objl: u16, attrs: &[(u16, &str)], geom: Geom| Feature { rcid: 0, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom };
    // A C-shaped bay, open to the east: its name goes inside it, not at its centre.
    let bay = vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 0.2], [0.2, 0.2], [0.2, 0.8], [1.0, 0.8], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]];
    let cell = Cell {
        name: "T".into(),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl: 50_000,
        features: vec![
            f(obj::SEAARE, &[(attr::OBJNAM, "Anse des Blancs Sablons")], Geom::Area(bay.clone())),
            f(obj::LNDRGN, &[(attr::OBJNAM, "Pointe de Corsen"), (attr::SCAMIN, "119999")], Geom::Point([0.5, 0.5])),
            f(obj::BUAARE, &[], Geom::Point([0.6, 0.6])),
            f(obj::LIGHTS, &[(attr::OBJNAM, "Phare")], Geom::Point([0.7, 0.7])),
            f(obj::UWTROC, &[(attr::OBJNAM, "Les Trois Pierres")], Geom::Point([0.8, 0.8])),
            // A wreck's `OBJNAM` is the vessel and what it was, not a name a chart prints.
            f(obj::WRECKS, &[(attr::OBJNAM, "F/V Tracey Ann (65 ft steel trawler)")], Geom::Point([0.9, 0.9])),
            f(obj::SEAARE, &[(attr::OBJNAM, "Boston Harbor Traffic Separation Scheme")], Geom::Area(bay.clone())),
        ],
        bounds: Bounds::EMPTY,
        warnings: vec![],
        skipped_updates: vec![],
    };
    let names = names_of(&cell);
    let got: Vec<_> = names.iter().map(|n| (n.text.as_str(), n.kind)).collect();
    assert_eq!(got, vec![("Anse des Blancs Sablons", NameKind::Sea), ("Pointe de Corsen", NameKind::Land), ("Les Trois Pierres", NameKind::Danger), ("Boston Harbor Traffic Separation Scheme", NameKind::Sea)]);
    assert!(names[3].spine.is_none() && names[0].spine.is_some(), "a traffic scheme is no sea to spread a name over");
    assert!(crate::core::vector::ring_contains(&bay[0], names[0].pos), "{:?}", names[0].pos);
    // SCAMIN decides when it is given; otherwise rocks wait for a close view and areas for room.
    let shows = |n: &CellName, denom: f64, zoom: f64, px_per_deg: f64| name_shows(n, 50_000, denom, zoom, px_per_deg, 1000.0);
    let cape = &names[1];
    assert!(shows(cape, 100_000.0, 11.0, 1.0) && !shows(cape, 150_000.0, 11.0, 1.0));
    let rock = &names[2];
    assert!(!shows(rock, 10_000.0, 12.0, 1.0) && shows(rock, 10_000.0, 13.0, 1.0));
    let bay = &names[0];
    assert!(!shows(bay, 1e6, 8.0, 10.0) && shows(bay, 1e6, 8.0, 100.0), "1 degree across is 10 px, then 100 px");
    assert!(!shows(bay, 1e6, 8.0, 2000.0), "wider than the view: the view is in it");
    assert!(!shows(rock, 2_000.0, 17.0, 1.0), "the cell is too far overscaled");
}

#[test]
fn a_sea_area_has_a_spine_over_its_water_and_land_over_its_land() {
    use std::collections::BTreeMap;
    let f = |objl: u16, attrs: &[(u16, &str)], geom: Geom| Feature { rcid: 0, objl, attrs: attrs.iter().map(|(k, v)| (*k, v.to_string())).collect::<BTreeMap<_, _>>(), geom };
    // A sea area 0.4 by 0.1 degrees, the northern half of which is land: the spine is in the southern half.
    let sea = vec![vec![[0.0, 0.0], [0.4, 0.0], [0.4, 0.1], [0.0, 0.1]]];
    let land = vec![vec![[-0.1, 0.05], [0.5, 0.05], [0.5, 0.2], [-0.1, 0.2]]];
    // An island 0.3 by 0.08 degrees.
    let island = vec![vec![[1.0, 0.0], [1.3, 0.0], [1.3, 0.08], [1.0, 0.08]]];
    let small = vec![vec![[2.0, 0.0], [2.001, 0.0], [2.001, 0.001], [2.0, 0.001]]];
    let cell = Cell {
        name: "T".into(),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl: 50_000,
        features: vec![
            f(obj::SEAARE, &[(attr::OBJNAM, "Chenal de Test")], Geom::Area(sea)),
            f(obj::LNDARE, &[], Geom::Area(land)),
            f(obj::LNDARE, &[(attr::OBJNAM, "Ile Longue")], Geom::Area(island)),
            f(obj::SEAARE, &[(attr::OBJNAM, "Trou")], Geom::Area(small)),
        ],
        bounds: Bounds::EMPTY,
        warnings: vec![],
        skipped_updates: vec![],
    };
    let names = names_of(&cell);
    let get = |t: &str| names.iter().find(|n| n.text == t).unwrap();
    let (x0, x1) = (0.0f64, 0.4f64);
    let sea = get("Chenal de Test").spine.as_ref().expect("a spine for a sea area");
    let lat_of = |p: &[f64; 2]| crate::core::geo::unproject(0.0, p[1], 0.0).lat;
    let _ = (x0, x1);
    assert!(sea.pts.iter().all(|p| lat_of(p) < 0.055), "the spine keeps to the water, off the land to the north: {:?}", sea.pts.iter().map(lat_of).collect::<Vec<_>>());
    assert!(sea.pts[0][0] < sea.pts.last().unwrap()[0]);
    assert!(get("Ile Longue").spine.is_some(), "an island is set along its land");
    assert!(get("Trou").spine.is_none(), "an area under 0.01 degrees is named level");
}

#[test]
fn names_with_a_spine_are_set_along_it_and_the_weaker_of_two_that_collide_is_left_level() {
    let spine = |y: f64| Arc::new(Spine { pts: vec![[100.0, y], [100.4, y]], clear: vec![0.05, 0.05] });
    let name = |text: &str, kind: NameKind, extent: f64, y: f64| CellName { text: text.into(), kind, pos: [0.0, 0.0], scamin: None, extent, spine: Some(spine(y)), out_from: None };
    let (strong, weak, apart) = (name("MER DE TEST", NameKind::Sea, 1.0, 100.0), name("CHENAL", NameKind::Channel, 0.2, 100.0), name("AUTRE", NameKind::Channel, 0.2, 100.1));
    let (kept, rest) = set_along_spines(vec![(&weak, 0.0, 0.0), (&strong, 0.0, 0.0), (&apart, 0.0, 0.0)], 12.0, &[]);
    let kept_names: Vec<&str> = kept.iter().map(|(n, _)| n.text.as_str()).collect();
    assert!(kept_names.contains(&"MER DE TEST") && kept_names.contains(&"AUTRE"), "{kept_names:?}");
    assert!(!kept_names.contains(&"CHENAL"), "it is in the sea's way");
    assert_eq!(rest.iter().map(|(n, _, _)| n.text.as_str()).collect::<Vec<_>>(), vec!["CHENAL"]);
    let (_, l) = kept.iter().find(|(n, _)| n.text == "MER DE TEST").unwrap();
    assert!(l.style.caps && l.glyphs.len() == "MER DE TEST".chars().count());
    // A name with no spine is not set along one.
    let level = CellName { spine: None, ..name("PLAT", NameKind::Sea, 1.0, 0.0) };
    assert_eq!(set_along_spines(vec![(&level, 0.0, 0.0)], 12.0, &[]).0.len(), 0);
}

#[test]
fn a_name_is_not_set_along_a_piece_when_a_bigger_piece_of_the_same_name_is_close() {
    let spine = |y: f64| Arc::new(Spine { pts: vec![[100.0, y], [100.4, y]], clear: vec![0.05, 0.05] });
    let piece = |extent: f64, lon: f64, y: f64| CellName { text: "VIRGINIA".into(), kind: NameKind::Land, pos: [lon, 0.0], scamin: None, extent, spine: Some(spine(y)), out_from: None };
    let (big, small, far) = (piece(1.0, 0.0, 100.0), piece(0.3, 0.001, 100.2), piece(0.3, 90.0, 100.3));
    let all = [&big, &small, &far];
    let (kept, _) = set_along_spines(vec![(&small, 0.0, 0.0), (&big, 0.0, 0.0), (&far, 0.0, 0.0)], 12.0, &all);
    let extents: Vec<f64> = kept.iter().map(|(n, _)| n.extent).collect();
    assert_eq!(kept.len(), 2, "the small piece next to the big one goes, the one far off stays: {extents:?}");
    assert!(kept.iter().any(|(n, _)| n.extent == 1.0) && kept.iter().any(|(n, _)| n.pos[0] == 90.0));
}

#[test]
fn a_cell_has_data_inside_its_coverage_or_its_bounds_when_it_has_none() {
    let square = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
    let mut e = CellSummary {
        name: "T".into(),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl: 50_000,
        bounds: Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 2.0, max_lat: 2.0 },
        features: 0,
        warnings: 0,
        cover: Default::default(),
    };
    assert!(has_data(&e, [1.5, 1.5]) && !has_data(&e, [2.5, 1.5]));
    e.cover.covered = vec![square];
    assert!(has_data(&e, [0.5, 0.5]) && !has_data(&e, [1.5, 1.5]));
}

#[test]
fn cells_are_kept_coarse_first() {
    use std::collections::BTreeMap;
    let mk = |cscl: u32| Cell {
        name: format!("C{cscl}"),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl,
        features: vec![],
        bounds: Bounds::EMPTY,
        warnings: vec![],
        skipped_updates: vec![],
    };
    let layer = EncLayer::new(Rc::new(|| {}));
    for s in [25_000, 1_500_000, 90_000] {
        layer.add_cell(mk(s));
    }
    let order: Vec<u32> = layer.summaries().iter().map(|c| c.cscl).collect();
    assert_eq!(order, vec![1_500_000, 90_000, 25_000]);
    let rev = layer.revision();
    layer.set_safety_depth(10.0);
    assert!(layer.revision() > rev, "the cached image is invalidated");
    let _ = BTreeMap::<u8, u8>::new();
}

#[test]
fn cells_from_a_folder_are_known_before_they_are_read() {
    let mk = |name: &str, update: u32, cscl: u32| CellSummary {
        name: name.into(),
        edition: 1,
        update,
        issue_date: String::new(),
        cscl,
        bounds: Bounds { min_lon: 0.0, min_lat: 0.0, max_lon: 1.0, max_lat: 1.0 },
        features: 10,
        warnings: 0,
        cover: Default::default(),
    };
    let layer = EncLayer::new(Rc::new(|| {}));
    layer.add_indexed(vec![
        (PathBuf::from("a/US5AAAAA.000"), mk("US5AAAAA.000", 0, 20_000)),
        (PathBuf::from("a/US2BBBBB.000"), mk("US2BBBBB.000", 0, 500_000)),
        (PathBuf::from("b/US5AAAAA.000"), mk("US5AAAAA.000", 0, 20_000)),
    ]);
    assert_eq!(layer.summaries().iter().map(|c| c.cscl).collect::<Vec<_>>(), vec![500_000, 20_000], "the copy is skipped; coarse first");
    let rev = layer.revision();
    layer.add_indexed(vec![(PathBuf::from("b/US5AAAAA.000"), mk("US5AAAAA.000", 0, 20_000))]);
    assert_eq!(layer.revision(), rev, "nothing new, nothing to redraw");
    assert_eq!(layer.exclusions().len(), 2);
}

#[test]
fn the_cells_drawn_longest_ago_leave_memory_first() {
    let layer = EncLayer::new(Rc::new(|| {}));
    let mk = |i: usize| CellSummary {
        name: format!("US5{i:05}.000"),
        edition: 1,
        update: 0,
        issue_date: String::new(),
        cscl: 20_000,
        bounds: Bounds::EMPTY,
        features: 0,
        warnings: 0,
        cover: Default::default(),
    };
    let total = KEEP_LOADED + 10;
    layer.add_indexed((0..total).map(|i| (PathBuf::from(format!("c/{i}.000")), mk(i))).collect());
    let cell = |name: String| Cell { name, edition: 1, update: 0, issue_date: String::new(), cscl: 20_000, features: vec![], bounds: Bounds::EMPTY, warnings: vec![], skipped_updates: vec![] };
    layer.draws.set(100);
    for (i, e) in layer.cells.borrow().iter().enumerate() {
        *e.cell.borrow_mut() = Some(Arc::new(Loaded::new(cell(e.meta.name.clone()))));
        // The first ten were drawn long ago; the next ten in the last draw.
        e.used.set(if i < 10 { 1 } else if i < 20 { 100 } else { 50 });
    }
    layer.evict();
    let cells = layer.cells.borrow();
    let held = cells.iter().filter(|e| e.cell.borrow().is_some()).count();
    assert_eq!(held, KEEP_LOADED);
    assert!(cells[..10].iter().all(|e| e.cell.borrow().is_none()), "the oldest are gone");
    assert!(cells[10..20].iter().all(|e| e.cell.borrow().is_some()), "the last draw's stay");
}

#[test]
fn a_light_is_built_from_s57_attributes() {
    use std::collections::BTreeMap;
    let f = Feature {
        rcid: 1,
        objl: obj::LIGHTS,
        attrs: BTreeMap::from([
            (attr::COLOUR, "1,3".to_string()),
            (attr::LITCHR, "2".to_string()),
            (attr::SIGGRP, "(2)".to_string()),
            (attr::SIGPER, "6".to_string()),
            (attr::VALNMR, "12".to_string()),
            (attr::SECTR1, "30".to_string()),
            (attr::SECTR2, "90".to_string()),
        ]),
        geom: Geom::Point([-4.5, 48.3]),
    };
    let l = light_of(&f, [-4.5, 48.3]);
    assert_eq!(crate::core::chart::aids::light_description(&l), "Fl(2)WR 6s 12M");
    assert_eq!(l.sector, Some((30.0, 90.0)));
    assert_eq!(l.pos, LatLon::new(48.3, -4.5));
}

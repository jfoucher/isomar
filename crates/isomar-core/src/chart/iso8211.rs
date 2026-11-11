//! ISO/IEC 8211: the container format of S-57 (and S-101) chart cells.
//!
//! A file starts with a data descriptive record (DDR) that says how each
//! field is laid out, followed by data records. Fields are decoded using the
//! formats the DDR declares, so this reader knows nothing about S-57 itself.

use std::collections::HashMap;

const UNIT_TERMINATOR: u8 = 0x1F;
const FIELD_TERMINATOR: u8 = 0x1E;

#[derive(Debug, PartialEq, Eq)]
pub enum IsoError {
    Truncated,
    BadLeader(String),
    BadDirectory(String),
    BadFormat(String),
}

impl std::fmt::Display for IsoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsoError::Truncated => write!(f, "the file ends in the middle of a record"),
            IsoError::BadLeader(s) => write!(f, "bad record leader: {s}"),
            IsoError::BadDirectory(s) => write!(f, "bad record directory: {s}"),
            IsoError::BadFormat(s) => write!(f, "bad field format: {s}"),
        }
    }
}

impl std::error::Error for IsoError {}

/// One subfield's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    /// Text; `None` runs to the next unit terminator.
    Text(Option<usize>),
    Int(Option<usize>),
    Real(Option<usize>),
    /// Unsigned little-endian integer of this many bytes (`b11`, `b12`, `b14`).
    UInt(usize),
    /// Signed little-endian integer (`b21`, `b22`, `b24`).
    SInt(usize),
    /// Raw bytes (`B(40)` is five).
    Bytes(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    Real(f64),
    Bytes(Vec<u8>),
}

impl Value {
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Real(r) => Some(*r as i64),
            Value::Text(t) => t.trim().parse().ok(),
            Value::Bytes(_) => None,
        }
    }

    pub fn text(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Int(i) => i.to_string(),
            Value::Real(r) => r.to_string(),
            Value::Bytes(b) => decode_text(b),
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Value::Bytes(b) => b,
            _ => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    pub tag: String,
    pub name: String,
    pub labels: Vec<String>,
    pub formats: Vec<Fmt>,
    /// The array descriptor starts with `*`: the subfields repeat.
    pub repeating: bool,
}

#[derive(Debug, Clone)]
pub struct Record {
    /// Field tag and its raw bytes, in file order.
    pub fields: Vec<(String, Vec<u8>)>,
}

impl Record {
    pub fn get(&self, tag: &str) -> Option<&[u8]> {
        self.fields.iter().find(|(t, _)| t == tag).map(|(_, d)| d.as_slice())
    }

    pub fn has(&self, tag: &str) -> bool {
        self.get(tag).is_some()
    }
}

#[derive(Debug, Default)]
pub struct Ddf {
    pub defs: HashMap<String, FieldDef>,
}

/// Parses a format string such as `(b11,b14,2A,A(8),A)` into one entry per
/// subfield, expanding repeat counts.
pub fn parse_formats(s: &str) -> Result<Vec<Fmt>, IsoError> {
    let s = s.trim();
    let inner = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')).unwrap_or(s);
    let mut items = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                items.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(&inner[start..]);
    let mut out = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let digits: String = item.chars().take_while(|c| c.is_ascii_digit()).collect();
        let rest = &item[digits.len()..];
        // `2b24` is two b24s, but `b24` alone starts with a letter.
        let count: usize = if digits.is_empty() { 1 } else { digits.parse().map_err(|_| IsoError::BadFormat(item.into()))? };
        let one: Vec<Fmt> = if rest.starts_with('(') {
            parse_formats(rest)?
        } else {
            vec![parse_one(rest)?]
        };
        for _ in 0..count {
            out.extend(one.iter().copied());
        }
    }
    Ok(out)
}

fn parse_one(s: &str) -> Result<Fmt, IsoError> {
    let bad = || IsoError::BadFormat(s.to_string());
    let width = |rest: &str| -> Result<Option<usize>, IsoError> {
        match rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
            Some(n) => n.parse().map(Some).map_err(|_| bad()),
            None if rest.is_empty() => Ok(None),
            None => Err(bad()),
        }
    };
    let mut ch = s.chars();
    match ch.next().ok_or_else(bad)? {
        'A' => Ok(Fmt::Text(width(&s[1..])?)),
        'I' => Ok(Fmt::Int(width(&s[1..])?)),
        'R' => Ok(Fmt::Real(width(&s[1..])?)),
        'B' => {
            let bits = width(&s[1..])?.ok_or_else(bad)?;
            Ok(Fmt::Bytes(bits.div_ceil(8)))
        }
        'b' => {
            let d = &s[1..];
            let (kind, w) = d.split_at(1.min(d.len()));
            let w: usize = w.parse().map_err(|_| bad())?;
            match kind {
                "1" if matches!(w, 1 | 2 | 4 | 8) => Ok(Fmt::UInt(w)),
                "2" if matches!(w, 1 | 2 | 4 | 8) => Ok(Fmt::SInt(w)),
                _ => Err(bad()),
            }
        }
        _ => Err(bad()),
    }
}

fn digits(b: &[u8]) -> Result<usize, IsoError> {
    std::str::from_utf8(b).ok().and_then(|s| s.trim().parse().ok()).ok_or_else(|| IsoError::BadLeader(String::from_utf8_lossy(b).into_owned()))
}

/// Splits a record into (tag, bytes) pairs. Returns the record and its length.
type RawRecord = (Vec<(String, Vec<u8>)>, usize, u8);

fn read_record_raw(data: &[u8]) -> Result<RawRecord, IsoError> {
    if data.len() < 24 {
        return Err(IsoError::Truncated);
    }
    let len = digits(&data[0..5])?;
    if len < 24 || len > data.len() {
        return Err(IsoError::Truncated);
    }
    let leader_id = data[6];
    let base = digits(&data[12..17])?;
    let (flen, fpos, ftag) = (digits(&data[20..21])?, digits(&data[21..22])?, digits(&data[23..24])?);
    if flen == 0 || fpos == 0 || ftag == 0 || base > len {
        return Err(IsoError::BadDirectory("entry sizes".into()));
    }
    let entry = ftag + flen + fpos;
    let dir = &data[24..base];
    let mut fields = Vec::new();
    let mut i = 0;
    while i + entry <= dir.len() && dir[i] != FIELD_TERMINATOR {
        let tag = String::from_utf8_lossy(&dir[i..i + ftag]).into_owned();
        let l = digits(&dir[i + ftag..i + ftag + flen])?;
        let p = digits(&dir[i + ftag + flen..i + entry])?;
        let (a, b) = (base + p, base + p + l);
        if b > len {
            return Err(IsoError::BadDirectory(format!("field {tag} runs past the record")));
        }
        fields.push((tag, data[a..b].to_vec()));
        i += entry;
    }
    Ok((fields, len, leader_id))
}

impl Ddf {
    fn add_definition(&mut self, tag: &str, data: &[u8]) -> Result<(), IsoError> {
        if tag == "0000" || data.len() < 9 {
            return Ok(());
        }
        // 9 bytes of field controls, then name, array descriptor, format controls.
        let body = &data[9..];
        let body = body.strip_suffix(&[FIELD_TERMINATOR]).unwrap_or(body);
        let mut parts = body.split(|b| *b == UNIT_TERMINATOR);
        let name = String::from_utf8_lossy(parts.next().unwrap_or(&[])).into_owned();
        let labels_raw = String::from_utf8_lossy(parts.next().unwrap_or(&[])).into_owned();
        let fmt_raw = String::from_utf8_lossy(parts.next().unwrap_or(&[])).into_owned();
        let repeating = labels_raw.starts_with('*');
        let labels: Vec<String> = labels_raw.trim_start_matches('*').split('!').filter(|s| !s.is_empty()).map(String::from).collect();
        let formats = if fmt_raw.is_empty() { Vec::new() } else { parse_formats(&fmt_raw)? };
        self.defs.insert(tag.to_string(), FieldDef { tag: tag.to_string(), name, labels, formats, repeating });
        Ok(())
    }

    /// Decodes a field's data into rows of subfield values (one row unless
    /// the field repeats). Anything that does not fit the format ends the
    /// field early rather than failing.
    pub fn decode(&self, tag: &str, data: &[u8]) -> Vec<Vec<Value>> {
        let Some(def) = self.defs.get(tag) else { return Vec::new() };
        if def.formats.is_empty() {
            return Vec::new();
        }
        // Only the last byte is the terminator: a binary value (a coordinate
        // whose low byte is 0x1E) can hold the same byte anywhere before it.
        let end = data.strip_suffix(&[FIELD_TERMINATOR]).unwrap_or(data).len();
        let mut pos = 0;
        let mut rows = Vec::new();
        while pos < end {
            let mut row = Vec::with_capacity(def.formats.len());
            for f in &def.formats {
                match read_value(data, &mut pos, *f) {
                    Some(v) => row.push(v),
                    None => return rows,
                }
            }
            rows.push(row);
            if !def.repeating {
                break;
            }
        }
        rows
    }
}

/// Text as S-57 writes it: lexical level 0 or 1, ISO 8859-1, which producers
/// fill with Windows-1252 (SHOM's `Presqu’île` has a 0x92). UTF-8 is taken
/// as it is. National text (NATF, level 2) is UCS-2 and comes out garbled,
/// but nothing reads it.
pub fn decode_text(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    const CP1252: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    b.iter().map(|&c| if (0x80..0xa0).contains(&c) { CP1252[(c - 0x80) as usize] } else { c as char }).collect()
}

fn read_value(data: &[u8], pos: &mut usize, f: Fmt) -> Option<Value> {
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let s = data.get(*pos..*pos + n)?;
        *pos += n;
        Some(s)
    };
    let text = |pos: &mut usize, w: Option<usize>| -> Option<String> {
        match w {
            Some(n) => take(pos, n).map(|b| decode_text(b).trim_end().to_string()),
            None => {
                let rest = data.get(*pos..)?;
                let end = rest.iter().position(|b| *b == UNIT_TERMINATOR || *b == FIELD_TERMINATOR).unwrap_or(rest.len());
                let s = decode_text(&rest[..end]);
                *pos += end;
                if data.get(*pos) == Some(&UNIT_TERMINATOR) {
                    *pos += 1;
                }
                Some(s)
            }
        }
    };
    Some(match f {
        Fmt::Text(w) => Value::Text(text(pos, w)?),
        Fmt::Int(w) => Value::Int(text(pos, w)?.trim().parse().unwrap_or(0)),
        Fmt::Real(w) => Value::Real(text(pos, w)?.trim().parse().unwrap_or(0.0)),
        Fmt::Bytes(n) => Value::Bytes(take(pos, n)?.to_vec()),
        Fmt::UInt(w) => {
            let b = take(pos, w)?;
            Value::Int(b.iter().rev().fold(0u64, |a, x| (a << 8) | *x as u64) as i64)
        }
        Fmt::SInt(w) => {
            let b = take(pos, w)?;
            let u = b.iter().rev().fold(0u64, |a, x| (a << 8) | *x as u64);
            let shift = 64 - 8 * w as u32;
            Value::Int(((u << shift) as i64) >> shift)
        }
    })
}

/// Reads a whole file: its field definitions and every data record.
pub fn read_file(data: &[u8]) -> Result<(Ddf, Vec<Record>), IsoError> {
    let mut ddf = Ddf::default();
    let mut records = Vec::new();
    let mut pos = 0;
    let mut first = true;
    while pos < data.len() {
        // Some producers pad the end of a file.
        if data[pos..].iter().all(|b| *b == 0 || *b == b' ' || *b == b'\n' || *b == b'\r') {
            break;
        }
        let (fields, len, leader_id) = read_record_raw(&data[pos..])?;
        pos += len;
        if first || leader_id == b'L' {
            for (tag, d) in &fields {
                ddf.add_definition(tag, d)?;
            }
            first = false;
        } else {
            records.push(Record { fields });
        }
    }
    Ok((ddf, records))
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Builds ISO 8211 files in memory.

    fn pad(n: usize, w: usize) -> String {
        format!("{n:0w$}")
    }

    /// A record from (tag, bytes) pairs, with a 4-byte tag, 4-digit length and position.
    pub fn record(leader_id: u8, fields: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut dir = Vec::new();
        let mut area: Vec<u8> = Vec::new();
        for (tag, data) in fields {
            dir.extend(tag.as_bytes());
            dir.extend(pad(data.len(), 4).as_bytes());
            dir.extend(pad(area.len(), 4).as_bytes());
            area.extend(data);
        }
        dir.push(0x1E);
        let base = 24 + dir.len();
        let total = base + area.len();
        let mut out = Vec::new();
        out.extend(pad(total, 5).as_bytes());
        out.push(b'3');
        out.push(leader_id);
        out.push(b'E');
        out.push(b'1');
        out.push(b' ');
        out.extend(b"  "); // field control length (unused here)
        out.extend(pad(base, 5).as_bytes());
        out.extend(b" ! ");
        out.extend(b"444");
        out.push(b'4');
        out.extend(area);
        // Fix the directory into place.
        let mut full = out[..24].to_vec();
        full.extend(dir);
        full.extend(&out[24..]);
        full
    }

    /// A field definition as the DDR carries it.
    pub fn definition(name: &str, labels: &str, formats: &str) -> Vec<u8> {
        let mut v = b"1600;&   ".to_vec();
        v.extend(name.as_bytes());
        v.push(0x1F);
        v.extend(labels.as_bytes());
        v.push(0x1F);
        v.extend(formats.as_bytes());
        v.push(0x1E);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn formats_expand_counts_and_groups() {
        assert_eq!(parse_formats("(b11,b14)").unwrap(), vec![Fmt::UInt(1), Fmt::UInt(4)]);
        assert_eq!(parse_formats("(2b24)").unwrap(), vec![Fmt::SInt(4), Fmt::SInt(4)]);
        assert_eq!(parse_formats("(3A,A(8),A)").unwrap(), vec![Fmt::Text(None), Fmt::Text(None), Fmt::Text(None), Fmt::Text(Some(8)), Fmt::Text(None)]);
        assert_eq!(parse_formats("(B(40),b11)").unwrap(), vec![Fmt::Bytes(5), Fmt::UInt(1)]);
        assert_eq!(parse_formats("(I(3),R(5))").unwrap(), vec![Fmt::Int(Some(3)), Fmt::Real(Some(5))]);
        assert_eq!(parse_formats("(2(b11,A))").unwrap(), vec![Fmt::UInt(1), Fmt::Text(None), Fmt::UInt(1), Fmt::Text(None)]);
        assert_eq!(parse_formats("b12").unwrap(), vec![Fmt::UInt(2)]);
    }

    #[test]
    fn text_is_utf8_or_else_windows_1252() {
        assert_eq!(decode_text("Presqu’île".as_bytes()), "Presqu’île");
        assert_eq!(decode_text(b"Presqu\x92\xeele de Kermorvan"), "Presqu’île de Kermorvan");
        assert_eq!(decode_text(b"Basse de la Ch\xe8vre"), "Basse de la Chèvre");
    }

    #[test]
    fn bad_formats_are_errors() {
        assert!(parse_formats("(Z)").is_err());
        assert!(parse_formats("(b13)").is_err());
        assert!(parse_formats("(A(x))").is_err());
        assert!(parse_formats("(B)").is_err());
    }

    fn ddr() -> Vec<u8> {
        record(b'L', &[
            ("0000", b"0000;&   ".to_vec()),
            ("VRID", definition("Vector record", "RCNM!RCID!RVER!RUIN", "(b11,b14,b12,b11)")),
            ("SG2D", definition("2-D coordinate", "*YCOO!XCOO", "(2b24)")),
            ("ATTF", definition("Feature attributes", "*ATTL!ATVL", "(b12,A)")),
        ])
    }

    #[test]
    fn decodes_fixed_and_repeating_fields() {
        let mut vrid = vec![130u8];
        vrid.extend(7u32.to_le_bytes());
        vrid.extend(2u16.to_le_bytes());
        vrid.push(1);
        vrid.push(0x1E);
        let mut sg = Vec::new();
        for (y, x) in [(480_000_000i32, -45_000_000i32), (-1, 2)] {
            sg.extend(y.to_le_bytes());
            sg.extend(x.to_le_bytes());
        }
        sg.push(0x1E);
        let mut attf = Vec::new();
        attf.extend(116u16.to_le_bytes());
        attf.extend(b"Les Pierres Noires");
        attf.push(0x1F);
        attf.extend(75u16.to_le_bytes());
        attf.extend(b"3,1");
        attf.push(0x1F);
        attf.push(0x1E);
        let mut file = ddr();
        file.extend(record(b'D', &[("VRID", vrid), ("SG2D", sg), ("ATTF", attf)]));
        let (ddf, recs) = read_file(&file).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        let v = &ddf.decode("VRID", r.get("VRID").unwrap())[0];
        assert_eq!((v[0].int(), v[1].int(), v[2].int(), v[3].int()), (Some(130), Some(7), Some(2), Some(1)));
        let pts = ddf.decode("SG2D", r.get("SG2D").unwrap());
        assert_eq!(pts.len(), 2);
        assert_eq!((pts[0][0].int(), pts[0][1].int()), (Some(480_000_000), Some(-45_000_000)));
        assert_eq!((pts[1][0].int(), pts[1][1].int()), (Some(-1), Some(2)), "signed values");
        let at = ddf.decode("ATTF", r.get("ATTF").unwrap());
        assert_eq!(at.len(), 2);
        assert_eq!(at[0][1].text(), "Les Pierres Noires");
        assert_eq!((at[1][0].int(), at[1][1].text().as_str()), (Some(75), "3,1"));
        assert_eq!(ddf.defs["SG2D"].labels, vec!["YCOO", "XCOO"]);
        assert!(ddf.defs["SG2D"].repeating && !ddf.defs["VRID"].repeating);
        assert_eq!(ddf.defs["VRID"].name, "Vector record");
    }

    #[test]
    fn a_binary_value_may_hold_the_terminator_byte() {
        // 0x1E is 30: this y coordinate starts with it, and the point is not the end of the field.
        let mut sg = Vec::new();
        for (y, x) in [(1, 2), (30, 3), (4, 5)] {
            sg.extend(i32::to_le_bytes(y));
            sg.extend(i32::to_le_bytes(x));
        }
        sg.push(0x1E);
        let mut file = ddr();
        file.extend(record(b'D', &[("SG2D", sg)]));
        let (ddf, recs) = read_file(&file).unwrap();
        let pts = ddf.decode("SG2D", recs[0].get("SG2D").unwrap());
        assert_eq!(pts.iter().map(|p| (p[0].int(), p[1].int())).collect::<Vec<_>>(), [(Some(1), Some(2)), (Some(30), Some(3)), (Some(4), Some(5))]);
    }

    #[test]
    fn truncated_and_corrupt_files_do_not_panic() {
        let mut file = ddr();
        file.extend(record(b'D', &[("VRID", vec![130, 1, 0, 0, 0, 1, 0, 1, 0x1E])]));
        for cut in [0, 10, 23, 30, file.len() - 5, file.len() - 1] {
            let r = read_file(&file[..cut]);
            assert!(r.is_err() || r.is_ok(), "no panic at {cut}");
        }
        assert!(read_file(&file[..file.len() - 3]).is_err(), "a cut record is an error");
        let mut bad = file.clone();
        bad[0..5].copy_from_slice(b"zzzzz");
        assert!(matches!(read_file(&bad), Err(IsoError::BadLeader(_))));
        assert_eq!(read_file(&[]).unwrap().1.len(), 0);
    }

    #[test]
    fn short_data_ends_a_field_early() {
        let (ddf, _) = read_file(&ddr()).unwrap();
        // Only half a coordinate pair.
        assert!(ddf.decode("SG2D", &[1, 0, 0, 0, 2, 0]).is_empty());
        assert!(ddf.decode("NOPE", &[1]).is_empty());
        assert!(ddf.decode("SG2D", &[0x1E]).is_empty());
    }

    #[test]
    fn real_cells_parse_when_present() {
        let p = std::path::Path::new("testdata/shom-iroise-s57/S57_OPENDATA_IROISEE/FR571220/FR571220.000");
        if !p.exists() {
            return;
        }
        let (ddf, recs) = read_file(&std::fs::read(p).unwrap()).unwrap();
        for tag in ["DSID", "DSPM", "VRID", "FRID", "FSPT", "ATTF", "SG2D", "VRPT"] {
            assert!(ddf.defs.contains_key(tag), "{tag}");
        }
        assert!(recs.len() > 1000, "{}", recs.len());
        let dspm = recs.iter().find(|r| r.has("DSPM")).unwrap();
        let row = &ddf.decode("DSPM", dspm.get("DSPM").unwrap())[0];
        // Subfields: RCNM RCID HDAT VDAT SDAT CSCL DUNI HUNI PUNI COUN COMF SOMF.
        assert_eq!(row[5].int(), Some(25_036), "compilation scale, {row:?}");
        assert!(row[10].int().is_some_and(|c| c > 0), "coordinate multiplication factor, {row:?}");
        assert_eq!(row[11].int(), Some(10), "sounding multiplication factor");
    }
}

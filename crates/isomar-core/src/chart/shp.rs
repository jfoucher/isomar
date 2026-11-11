//! A streaming ESRI shapefile (`.shp`) reader for points, polylines and
//! polygons. Coordinates come out as stored; reproject them yourself.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Null,
    Point([f64; 2]),
    /// Each part is a line; for polygons each part is a ring.
    Polyline(Vec<Vec<[f64; 2]>>),
    Polygon(Vec<Vec<[f64; 2]>>),
}

#[derive(Debug)]
pub enum ShpError {
    Io(io::Error),
    Format(String),
}

impl From<io::Error> for ShpError {
    fn from(e: io::Error) -> Self {
        ShpError::Io(e)
    }
}

impl std::fmt::Display for ShpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShpError::Io(e) => write!(f, "{e}"),
            ShpError::Format(s) => write!(f, "bad shapefile: {s}"),
        }
    }
}

impl std::error::Error for ShpError {}

pub struct ShpReader<R: Read> {
    r: R,
    /// Bytes of the file consumed so far, and its total length.
    pos: u64,
    len: u64,
    /// Shape type from the file header.
    pub shape_type: i32,
    /// `[xmin, ymin, xmax, ymax]` from the file header.
    pub bbox: [f64; 4],
}

impl ShpReader<BufReader<File>> {
    pub fn open(path: &Path) -> Result<Self, ShpError> {
        let f = File::open(path)?;
        let len = f.metadata()?.len();
        ShpReader::new(BufReader::with_capacity(1 << 20, f), len)
    }
}

impl<R: Read> ShpReader<R> {
    pub fn new(mut r: R, len: u64) -> Result<Self, ShpError> {
        let mut h = [0u8; 100];
        r.read_exact(&mut h)?;
        if i32::from_be_bytes(h[0..4].try_into().unwrap()) != 9994 {
            return Err(ShpError::Format("missing file code 9994".into()));
        }
        let shape_type = i32::from_le_bytes(h[32..36].try_into().unwrap());
        let mut bbox = [0.0; 4];
        for (i, b) in bbox.iter_mut().enumerate() {
            *b = f64::from_le_bytes(h[36 + i * 8..44 + i * 8].try_into().unwrap());
        }
        Ok(ShpReader { r, pos: 100, len, shape_type, bbox })
    }

    /// Fraction of the file read, 0..1.
    pub fn progress(&self) -> f64 {
        if self.len == 0 { 1.0 } else { self.pos as f64 / self.len as f64 }
    }

    /// The next record, or `None` at the end of the file.
    pub fn next_shape(&mut self) -> Result<Option<Shape>, ShpError> {
        let mut rh = [0u8; 8];
        match self.r.read_exact(&mut rh) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let content_len = i32::from_be_bytes(rh[4..8].try_into().unwrap()) as usize * 2;
        let mut body = vec![0u8; content_len];
        self.r.read_exact(&mut body)?;
        self.pos += 8 + content_len as u64;
        parse_record(&body).map(Some)
    }
}

impl<R: Read> Iterator for ShpReader<R> {
    type Item = Result<Shape, ShpError>;
    fn next(&mut self) -> Option<Self::Item> {
        self.next_shape().transpose()
    }
}

fn f64_at(b: &[u8], o: usize) -> Result<f64, ShpError> {
    b.get(o..o + 8)
        .map(|s| f64::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| ShpError::Format("truncated record".into()))
}

fn i32_at(b: &[u8], o: usize) -> Result<i32, ShpError> {
    b.get(o..o + 4)
        .map(|s| i32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| ShpError::Format("truncated record".into()))
}

fn parse_record(b: &[u8]) -> Result<Shape, ShpError> {
    let kind = i32_at(b, 0)?;
    match kind {
        0 => Ok(Shape::Null),
        // Point, PointZ, PointM
        1 | 11 | 21 => Ok(Shape::Point([f64_at(b, 4)?, f64_at(b, 12)?])),
        // PolyLine, Polygon and their Z/M variants share the XY layout.
        3 | 5 | 13 | 15 | 23 | 25 => {
            let num_parts = i32_at(b, 36)? as usize;
            let num_points = i32_at(b, 40)? as usize;
            if num_parts > 1 << 24 || num_points > 1 << 28 {
                return Err(ShpError::Format("implausible part/point count".into()));
            }
            let mut starts = Vec::with_capacity(num_parts + 1);
            for i in 0..num_parts {
                starts.push(i32_at(b, 44 + i * 4)? as usize);
            }
            starts.push(num_points);
            let pts_off = 44 + num_parts * 4;
            let mut parts = Vec::with_capacity(num_parts);
            for w in starts.windows(2) {
                if w[0] > w[1] || w[1] > num_points {
                    return Err(ShpError::Format("part index out of range".into()));
                }
                let mut ring = Vec::with_capacity(w[1] - w[0]);
                for i in w[0]..w[1] {
                    let o = pts_off + i * 16;
                    ring.push([f64_at(b, o)?, f64_at(b, o + 8)?]);
                }
                parts.push(ring);
            }
            Ok(if matches!(kind, 3 | 13 | 23) { Shape::Polyline(parts) } else { Shape::Polygon(parts) })
        }
        k => Err(ShpError::Format(format!("unsupported shape type {k}"))),
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Builds tiny shapefiles in memory for tests.

    fn header(shape_type: i32, total_len: usize, bbox: [f64; 4]) -> Vec<u8> {
        let mut h = vec![0u8; 100];
        h[0..4].copy_from_slice(&9994i32.to_be_bytes());
        h[24..28].copy_from_slice(&((total_len / 2) as i32).to_be_bytes());
        h[28..32].copy_from_slice(&1000i32.to_le_bytes());
        h[32..36].copy_from_slice(&shape_type.to_le_bytes());
        for (i, v) in bbox.iter().enumerate() {
            h[36 + i * 8..44 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        h
    }

    fn record(num: i32, body: Vec<u8>) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&num.to_be_bytes());
        r.extend_from_slice(&((body.len() / 2) as i32).to_be_bytes());
        r.extend(body);
        r
    }

    pub fn polygon_body(rings: &[Vec<[f64; 2]>], kind: i32) -> Vec<u8> {
        let npts: usize = rings.iter().map(|r| r.len()).sum();
        let mut b = Vec::new();
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&[0u8; 32]); // bbox, not read
        b.extend_from_slice(&(rings.len() as i32).to_le_bytes());
        b.extend_from_slice(&(npts as i32).to_le_bytes());
        let mut start = 0;
        for r in rings {
            b.extend_from_slice(&(start as i32).to_le_bytes());
            start += r.len();
        }
        for r in rings {
            for p in r {
                b.extend_from_slice(&p[0].to_le_bytes());
                b.extend_from_slice(&p[1].to_le_bytes());
            }
        }
        b
    }

    pub fn point_body(p: [f64; 2]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&p[0].to_le_bytes());
        b.extend_from_slice(&p[1].to_le_bytes());
        b
    }

    pub fn file(shape_type: i32, bodies: Vec<Vec<u8>>) -> Vec<u8> {
        let recs: Vec<u8> = bodies.into_iter().enumerate().flat_map(|(i, b)| record(i as i32 + 1, b)).collect();
        let mut out = header(shape_type, 100 + recs.len(), [0.0; 4]);
        out.extend(recs);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn reads_polygons_with_holes() {
        let outer = vec![[0.0, 0.0], [0.0, 10.0], [10.0, 10.0], [10.0, 0.0], [0.0, 0.0]];
        let hole = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0], [4.0, 4.0]];
        let data = file(5, vec![polygon_body(&[outer.clone(), hole.clone()], 5)]);
        let len = data.len() as u64;
        let mut r = ShpReader::new(&data[..], len).unwrap();
        assert_eq!(r.shape_type, 5);
        assert_eq!(r.next_shape().unwrap(), Some(Shape::Polygon(vec![outer, hole])));
        assert_eq!(r.next_shape().unwrap(), None);
        assert!((r.progress() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn reads_points_and_polylines() {
        let line = vec![[1.0, 1.0], [2.0, 3.0]];
        let data = file(1, vec![point_body([3.5, -4.5]), polygon_body(std::slice::from_ref(&line), 3)]);
        let len = data.len() as u64;
        let shapes: Vec<_> = ShpReader::new(&data[..], len).unwrap().map(Result::unwrap).collect();
        assert_eq!(shapes, vec![Shape::Point([3.5, -4.5]), Shape::Polyline(vec![line])]);
    }

    #[test]
    fn rejects_other_files() {
        assert!(matches!(ShpReader::new(&[0u8; 100][..], 100), Err(ShpError::Format(_))));
    }

    #[test]
    fn truncated_record_is_an_error_not_a_panic() {
        let mut data = file(5, vec![polygon_body(&[vec![[0.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]], 5)]);
        data.truncate(data.len() - 20);
        let len = data.len() as u64;
        let mut r = ShpReader::new(&data[..], len).unwrap();
        assert!(r.next_shape().is_err());
    }

    #[test]
    fn corrupt_counts_are_rejected() {
        let mut body = polygon_body(&[vec![[0.0, 0.0], [1.0, 1.0], [0.0, 1.0]]], 5);
        body[40..44].copy_from_slice(&i32::MAX.to_le_bytes());
        let data = file(5, vec![body]);
        let len = data.len() as u64;
        assert!(ShpReader::new(&data[..], len).unwrap().next_shape().is_err());
    }
}

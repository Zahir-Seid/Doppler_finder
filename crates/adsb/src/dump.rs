//! adsb.lol `globe_history` daily-dump ingest (FR-DA-01, ADSB-05).
//!
//! Ported from `franken_doppler_finder` `fdop-data/src/ingest.rs` (MIT License
//! with OpenAI/Anthropic rider; see `docs/third_party.md`), adapted to this
//! crate's `FlightTrack`/`TrackSample` model: single-threaded instead of a
//! rayon pipeline, no geoid conversion (`geometric_altitude_m` is the
//! ellipsoidal height, so `alt_geom_ft` converts directly), no engine-class
//! filtering (the store keeps every aircraft; `legs` classifies at read
//! time), and no leg-boundary events beyond the flag bits.
//!
//! The archive is a tar — possibly several `.tar.a?` release parts
//! concatenated — of `trace_full_*.json` members. Each member is gunzipped by
//! magic (`1f 8b`, despite the `.json` name) and holds one aircraft's trace:
//!
//! ```text
//! {"icao": ..., "r": ..., "t": ..., "timestamp": <unix s>, "trace": [row, ...]}
//! ```
//!
//! readsb row: `[dt, lat, lon, alt_baro|"ground"|null, gs_kt, track, flags,
//! vrate_fpm, details|null, source, alt_geom_ft, geom_rate_fpm, ias, roll]`
//! where `dt` is seconds after `timestamp`. Fields from index 9 exist only in
//! files from 2022 on. Row flags: 1 stale, 2 new leg, 4 vrate geometric,
//! 8 altitude geometric.
//!
//! Rows inside the query window and region, or bracketing a segment that
//! crosses both, are kept with two neighbours on each side so interpolation
//! stays anchored at the edges. [`AdsbLolArchive`] implements [`TrackSource`]
//! over a local archive file or directory for offline replay; release
//! discovery and download are fetch-layer work.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use core_types::UtcMillis;
use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

use crate::error::AdsbError;
use crate::source::{RegionQuery, TrackSource};
use crate::tracks::{
    FLAG_GEOM_FROM_BARO, FLAG_MLAT, FLAG_NEW_LEG, FLAG_NEW_LEG_AFTER, FLAG_STALE, FLAG_TISB,
    FlightTrack, TrackSample, TrackSourceKind,
};

const FT_TO_M: f64 = 0.3048;
const KT_TO_MPS: f64 = 0.514_444;
const FPM_TO_MPS: f64 = 0.3048 / 60.0;
/// Neighbouring samples kept on each side of a kept run, so interpolation at
/// the window and region edges stays anchored.
const EDGE_NEIGHBOURS: usize = 2;

/// What to keep from a trace archive.
#[derive(Debug, Clone, Copy)]
pub struct DumpSpec {
    /// Southern boundary in degrees.
    pub min_lat_deg: f64,
    /// Western boundary in degrees.
    pub min_lon_deg: f64,
    /// Northern boundary in degrees.
    pub max_lat_deg: f64,
    /// Eastern boundary in degrees.
    pub max_lon_deg: f64,
    /// Window start, unix seconds, inclusive.
    pub t_start_s: f64,
    /// Window end, unix seconds, inclusive.
    pub t_end_s: f64,
}

impl DumpSpec {
    /// Build a spec from a validated [`RegionQuery`]. The query's millisecond
    /// bounds convert to the seconds the archive rows use; callers pad the
    /// query itself when they want archive padding.
    pub fn from_query(query: &RegionQuery) -> Self {
        Self {
            min_lat_deg: query.min_lat_deg,
            min_lon_deg: query.min_lon_deg,
            max_lat_deg: query.max_lat_deg,
            max_lon_deg: query.max_lon_deg,
            t_start_s: query.from.0 as f64 / 1000.0,
            t_end_s: query.to.0 as f64 / 1000.0,
        }
    }

    fn contains(&self, lat: f64, lon: f64) -> bool {
        lat >= self.min_lat_deg
            && lat <= self.max_lat_deg
            && lon >= self.min_lon_deg
            && lon <= self.max_lon_deg
    }

    /// Whether a line between two `(time, latitude, longitude)` samples enters
    /// both this box and the inclusive time window at the same fraction of
    /// the segment. This keeps interpolation anchors when neither reported
    /// point is inside a data-selection window; it does not create positions
    /// or replace the acoustic track interpolation. Longitudes follow the
    /// short way across the dateline; boxes themselves do not wrap.
    fn intersects_timed_segment(&self, a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        if ![a.0, a.1, a.2, b.0, b.1, b.2]
            .into_iter()
            .all(f64::is_finite)
            || b.0 <= a.0
            || !(b.0 - a.0).is_finite()
            || !(-90.0..=90.0).contains(&a.1)
            || !(-90.0..=90.0).contains(&b.1)
            || !(-180.0..=180.0).contains(&a.2)
            || !(-180.0..=180.0).contains(&b.2)
            || !(self.min_lat_deg <= self.max_lat_deg
                && self.min_lon_deg <= self.max_lon_deg
                && self.t_start_s <= self.t_end_s)
        {
            return false;
        }
        let clip = |span: &mut (f64, f64), start: f64, delta: f64, lo: f64, hi: f64| {
            if delta == 0.0 {
                return start >= lo && start <= hi;
            }
            let (u, v) = ((lo - start) / delta, (hi - start) / delta);
            span.0 = span.0.max(u.min(v));
            span.1 = span.1.min(u.max(v));
            span.0 <= span.1
        };
        let mut span = (0.0, 1.0);
        if !clip(&mut span, a.0, b.0 - a.0, self.t_start_s, self.t_end_s)
            || !clip(
                &mut span,
                a.1,
                b.1 - a.1,
                self.min_lat_deg,
                self.max_lat_deg,
            )
        {
            return false;
        }
        let mut dlon = b.2 - a.2;
        if dlon > 180.0 {
            dlon -= 360.0;
        } else if dlon < -180.0 {
            dlon += 360.0;
        }
        [-360.0, 0.0, 360.0].into_iter().any(|shift| {
            let mut shifted = span;
            clip(
                &mut shifted,
                a.2,
                dlon,
                self.min_lon_deg + shift,
                self.max_lon_deg + shift,
            )
        })
    }
}

/// Counts for one ingest pass, for the manifest and progress reporting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DumpStats {
    /// Archive bytes consumed.
    pub bytes: u64,
    /// `trace_full_*` members seen.
    pub members: u64,
    /// Members that failed to parse.
    pub parse_errors: u64,
    /// Tracks with at least one retained sample.
    pub tracks_kept: u64,
    /// Samples retained across all tracks.
    pub points_kept: u64,
}

/// One retained position in archive units (seconds, feet, knots).
#[derive(Clone, Copy, Debug)]
struct DumpPoint {
    /// Unix seconds.
    t: f64,
    lat: f64,
    lon: f64,
    /// Ellipsoidal height in metres, when reported.
    alt_geom_m: Option<f64>,
    /// Barometric altitude in metres, when reported.
    alt_baro_m: Option<f64>,
    gs_mps: Option<f64>,
    track_deg: Option<f64>,
    vrate_mps: Option<f64>,
    flags: u8,
    /// A readsb new-leg marker or ground report fell on this point or a
    /// dropped row before it.
    new_leg: bool,
}

/// One aircraft's retained points from one archive member.
#[derive(Debug)]
struct DumpAircraft {
    icao24: String,
    type_code: Option<String>,
    points: Vec<DumpPoint>,
}

// ------------------------------------------------------------------------------------------
// Typed, lenient JSON visitors (ported from fdop-data)
// ------------------------------------------------------------------------------------------

fn drain_seq<'de, A: SeqAccess<'de>>(mut a: A) -> Result<(), A::Error> {
    while a.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

fn drain_map<'de, A: MapAccess<'de>>(mut a: A) -> Result<(), A::Error> {
    while a.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

/// Number or anything else (-> None). Never fails on type mismatch.
#[derive(Clone, Copy, Debug, Default)]
struct LNum(Option<f64>);

impl<'de> Deserialize<'de> for LNum {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LNum;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number or null")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<LNum, E> {
                Ok(LNum(v.is_finite().then_some(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<LNum, E> {
                Ok(LNum(Some(v as f64)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<LNum, E> {
                Ok(LNum(Some(v as f64)))
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<LNum, E> {
                Ok(LNum(None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<LNum, E> {
                Ok(LNum(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<LNum, E> {
                Ok(LNum(None))
            }
            fn visit_none<E: de::Error>(self) -> Result<LNum, E> {
                Ok(LNum(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<LNum, A::Error> {
                drain_seq(a)?;
                Ok(LNum(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<LNum, A::Error> {
                drain_map(a)?;
                Ok(LNum(None))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Alt {
    #[default]
    Null,
    Ground,
    Num(f64),
}

impl<'de> Deserialize<'de> for Alt {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Alt;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an altitude, \"ground\" or null")
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Alt, E> {
                Ok(if v.is_finite() {
                    Alt::Num(v)
                } else {
                    Alt::Null
                })
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Alt, E> {
                Ok(Alt::Num(v as f64))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Alt, E> {
                Ok(Alt::Num(v as f64))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Alt, E> {
                Ok(if v == "ground" {
                    Alt::Ground
                } else {
                    Alt::Null
                })
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Alt, E> {
                Ok(Alt::Null)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Alt, E> {
                Ok(Alt::Null)
            }
            fn visit_none<E: de::Error>(self) -> Result<Alt, E> {
                Ok(Alt::Null)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Alt, A::Error> {
                drain_seq(a)?;
                Ok(Alt::Null)
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Alt, A::Error> {
                drain_map(a)?;
                Ok(Alt::Null)
            }
        }
        d.deserialize_any(V)
    }
}

/// Position source -> flag bits (MLAT / TIS-B), 0 otherwise.
#[derive(Clone, Copy, Debug, Default)]
struct Src(u8);

fn src_bits(s: &str) -> u8 {
    if s.contains("mlat") {
        FLAG_MLAT
    } else if s.contains("tisb") {
        FLAG_TISB
    } else {
        0
    }
}

impl<'de> Deserialize<'de> for Src {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Src;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a source string or null")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Src, E> {
                Ok(Src(src_bits(v)))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Src, E> {
                Ok(Src(0))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Src, E> {
                Ok(Src(0))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Src, E> {
                Ok(Src(0))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Src, E> {
                Ok(Src(0))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Src, E> {
                Ok(Src(0))
            }
            fn visit_none<E: de::Error>(self) -> Result<Src, E> {
                Ok(Src(0))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Src, A::Error> {
                drain_seq(a)?;
                Ok(Src(0))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Src, A::Error> {
                drain_map(a)?;
                Ok(Src(0))
            }
        }
        d.deserialize_any(V)
    }
}

/// Lenient optional string (non-strings -> None).
#[derive(Clone, Debug, Default)]
struct LStr(Option<String>);

impl<'de> Deserialize<'de> for LStr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = LStr;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string or null")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<LStr, E> {
                Ok(LStr(Some(v.to_string())))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<LStr, E> {
                Ok(LStr(Some(v)))
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_unit<E: de::Error>(self) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_none<E: de::Error>(self) -> Result<LStr, E> {
                Ok(LStr(None))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<LStr, A::Error> {
                drain_seq(a)?;
                Ok(LStr(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<LStr, A::Error> {
                drain_map(a)?;
                Ok(LStr(None))
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Row {
    dt: Option<f64>,
    lat: Option<f64>,
    lon: Option<f64>,
    alt: Alt,
    gs: Option<f64>,
    trk: Option<f64>,
    flags: i64,
    vrate: Option<f64>,
    src: u8,
    alt_geom: Option<f64>,
    geom_rate: Option<f64>,
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Row;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a readsb trace row array")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Row, A::Error> {
                let mut r = Row::default();
                macro_rules! next {
                    ($t:ty) => {
                        match a.next_element::<$t>()? {
                            Some(v) => v,
                            None => return Ok(r),
                        }
                    };
                }
                r.dt = next!(LNum).0;
                r.lat = next!(LNum).0;
                r.lon = next!(LNum).0;
                r.alt = next!(Alt);
                r.gs = next!(LNum).0;
                r.trk = next!(LNum).0;
                r.flags = next!(LNum).0.map_or(0, |f| f as i64);
                r.vrate = next!(LNum).0;
                let _details: IgnoredAny = next!(IgnoredAny);
                r.src = next!(Src).0;
                r.alt_geom = next!(LNum).0;
                r.geom_rate = next!(LNum).0;
                while a.next_element::<IgnoredAny>()?.is_some() {}
                Ok(r)
            }
            // A non-array row (should not happen) is skipped rather than failing the file.
            fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Row, A::Error> {
                drain_map(a)?;
                Ok(Row::default())
            }
            fn visit_unit<E: de::Error>(self) -> Result<Row, E> {
                Ok(Row::default())
            }
        }
        d.deserialize_any(V)
    }
}

#[derive(Debug, Default, Deserialize)]
struct TraceFile {
    #[serde(default)]
    icao: LStr,
    #[serde(default)]
    t: LStr,
    #[serde(default)]
    timestamp: LNum,
    #[serde(default)]
    trace: Vec<Row>,
}

// ------------------------------------------------------------------------------------------
// Member parsing
// ------------------------------------------------------------------------------------------

/// Outcome of parsing one member.
enum Parsed {
    Kept(DumpAircraft),
    NoOverlap,
    Error,
}

fn gunzip_if_needed(b: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, std::io::Error> {
    if b.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::with_capacity(b.len() * 6);
        flate2::read::MultiGzDecoder::new(b).read_to_end(&mut out)?;
        Ok(std::borrow::Cow::Owned(out))
    } else {
        Ok(std::borrow::Cow::Borrowed(b))
    }
}

/// Parse one `trace_full` member (gzip or plain JSON) and filter it.
fn parse_member(bytes: &[u8], name_hint: &str, spec: &DumpSpec) -> Parsed {
    let Ok(json) = gunzip_if_needed(bytes) else {
        return Parsed::Error;
    };
    let tf: TraceFile = match serde_json::from_slice(&json) {
        Ok(v) => v,
        Err(_) => return Parsed::Error,
    };
    let base = tf.timestamp.0.unwrap_or(0.0);
    let pts = trace_points(&tf.trace, base);
    let inside: Vec<bool> = pts
        .iter()
        .map(|p| p.t >= spec.t_start_s && p.t <= spec.t_end_s && spec.contains(p.lat, p.lon))
        .collect();
    let mut keep = inside.clone();
    for (i, pair) in pts.windows(2).enumerate() {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let (Some(&inside_a), Some(&inside_b)) = (inside.get(i), inside.get(i + 1)) else {
            continue;
        };
        if !inside_a
            && !inside_b
            && !b.new_leg
            && spec.intersects_timed_segment((a.t, a.lat, a.lon), (b.t, b.lat, b.lon))
        {
            if let Some(slot) = keep.get_mut(i) {
                *slot = true;
            }
            if let Some(next) = keep.get_mut(i + 1) {
                *next = true;
            }
        }
    }
    if !keep.iter().any(|k| *k) {
        return Parsed::NoOverlap;
    }
    let points = retain_with_neighbours(&pts, &keep, EDGE_NEIGHBOURS);
    let icao24 = tf
        .icao
        .0
        .clone()
        .unwrap_or_else(|| {
            name_hint
                .rsplit('/')
                .next()
                .unwrap_or(name_hint)
                .trim_start_matches("trace_full_")
                .trim_end_matches(".json")
                .to_string()
        })
        .trim()
        .to_ascii_lowercase();
    let type_code =
        tf.t.0
            .as_deref()
            .map(str::trim)
            .filter(|typ| !typ.is_empty())
            .map(str::to_ascii_uppercase);
    Parsed::Kept(DumpAircraft {
        icao24,
        type_code,
        points,
    })
}

/// Convert one member's readsb rows into retained points.
///
/// Rows without a time, position or any altitude are skipped; ground reports
/// never become points but mark a new-leg boundary on the surrounding
/// airborne samples. `geometric_altitude_m` is ellipsoidal, so the readsb
/// geometric feet convert directly; no geoid undulation is applied here.
fn trace_points(rows: &[Row], base: f64) -> Vec<DumpPoint> {
    let mut pts: Vec<DumpPoint> = Vec::with_capacity(rows.len());
    let mut pending_leg = false;
    for row in rows {
        if row.flags & 2 != 0 || row.alt == Alt::Ground {
            pending_leg = true;
            if let Some(previous) = pts.last_mut() {
                previous.flags |= FLAG_NEW_LEG_AFTER;
            }
        }
        let (Some(dt), Some(lat), Some(lon)) = (row.dt, row.lat, row.lon) else {
            continue;
        };
        if row.alt == Alt::Ground {
            continue;
        }
        let geom = match (row.alt_geom, row.alt) {
            (Some(g), _) => Some(g * FT_TO_M),
            (None, Alt::Num(a)) if row.flags & 8 != 0 => Some(a * FT_TO_M),
            _ => None,
        };
        let baro = match row.alt {
            Alt::Num(a) if row.flags & 8 == 0 => Some(a * FT_TO_M),
            _ => None,
        };
        if geom.is_none() && baro.is_none() {
            continue;
        }
        let t = base + dt;
        if !t.is_finite() {
            continue;
        }
        let mut flags = row.src;
        if row.flags & 1 != 0 {
            flags |= FLAG_STALE;
        }
        let vrate = row.geom_rate.or(row.vrate);
        pts.push(DumpPoint {
            t,
            lat,
            lon,
            alt_geom_m: geom,
            alt_baro_m: baro,
            gs_mps: row.gs.filter(|v| *v >= 0.0).map(|v| v * KT_TO_MPS),
            track_deg: row.trk,
            vrate_mps: vrate.map(|v| v * FPM_TO_MPS),
            flags,
            new_leg: std::mem::take(&mut pending_leg),
        });
    }
    pts
}

/// Keep flagged points plus `k` neighbours each side. A new-leg marker on a
/// dropped point is carried to the next retained point.
fn retain_with_neighbours(pts: &[DumpPoint], keep: &[bool], k: usize) -> Vec<DumpPoint> {
    let n = pts.len();
    let mut grown = vec![false; n];
    for (i, kept) in keep.iter().enumerate() {
        if *kept {
            let lo = i.saturating_sub(k);
            let hi = (i + k + 1).min(n);
            if let Some(slice) = grown.get_mut(lo..hi) {
                for g in slice {
                    *g = true;
                }
            }
        }
    }
    let mut out = Vec::with_capacity(grown.iter().filter(|g| **g).count());
    let mut carry = false;
    for (p, g) in pts.iter().zip(&grown) {
        if *g {
            let mut q = *p;
            q.new_leg |= carry;
            carry = false;
            out.push(q);
        } else {
            let boundary = p.new_leg || p.flags & FLAG_NEW_LEG_AFTER != 0;
            carry |= boundary;
            if boundary && let Some(previous) = out.last_mut() {
                previous.flags |= FLAG_NEW_LEG_AFTER;
            }
        }
    }
    out
}

fn accumulate(res: Parsed, aircraft: &mut BTreeMap<String, DumpAircraft>, st: &mut DumpStats) {
    match res {
        Parsed::Kept(a) => {
            st.tracks_kept += 1;
            st.points_kept += a.points.len() as u64;
            match aircraft.get_mut(&a.icao24) {
                Some(existing) => {
                    existing.points.extend(a.points);
                    if existing.type_code.is_none() {
                        existing.type_code = a.type_code;
                    }
                }
                None => {
                    aircraft.insert(a.icao24.clone(), a);
                }
            }
        }
        Parsed::NoOverlap => {}
        Parsed::Error => st.parse_errors += 1,
    }
}

fn aircraft_to_track(
    icao24: String,
    type_code: Option<String>,
    mut points: Vec<DumpPoint>,
) -> Option<FlightTrack> {
    points.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mut samples = Vec::with_capacity(points.len());
    for p in points {
        let mut flags = p.flags;
        if p.new_leg {
            flags |= FLAG_NEW_LEG;
        }
        // `geometric_altitude_m` is required; when the archive only reported
        // a barometric altitude, reuse it and mark the substitution.
        let (geom_m, geom_from_baro) = match (p.alt_geom_m, p.alt_baro_m) {
            (Some(geom), _) => (geom, false),
            (None, Some(baro)) => (baro, true),
            (None, None) => continue,
        };
        if geom_from_baro {
            flags |= FLAG_GEOM_FROM_BARO;
        }
        samples.push(TrackSample {
            utc: UtcMillis((p.t * 1000.0).round() as i64),
            lat_deg: p.lat,
            lon_deg: p.lon,
            geometric_altitude_m: geom_m,
            barometric_altitude_m: p.alt_baro_m,
            ground_speed_m_per_s: p.gs_mps,
            track_deg: p.track_deg,
            vertical_rate_m_per_s: p.vrate_mps,
            latency_s: None,
            flags,
        });
    }
    let track = FlightTrack {
        icao24,
        callsign: None,
        type_code,
        source: TrackSourceKind::AdsbLol,
        samples,
    };
    track.validate().ok()?;
    Some(track)
}

// ------------------------------------------------------------------------------------------
// Tar walking
// ------------------------------------------------------------------------------------------

struct CountingReader<R> {
    inner: R,
    count: Arc<AtomicU64>,
    saw_eof: bool,
}

impl<R: Read> CountingReader<R> {
    /// The tar iterator accepts bare EOF and stops after its first zero block.
    /// Require both end blocks, then drain padding through transport EOF
    /// before accepting the archive as complete.
    fn finish_tar(mut self) -> Result<(), AdsbError> {
        if self.saw_eof {
            return Err(AdsbError::Parse(
                "tar is missing the end-of-archive marker".to_owned(),
            ));
        }
        let mut block = [0u8; 512];
        self.read_exact(&mut block)
            .map_err(|e| AdsbError::Parse(format!("tar end marker: {e}")))?;
        if block.iter().any(|b| *b != 0) {
            return Err(AdsbError::Parse(
                "tar end marker contains nonzero bytes".to_owned(),
            ));
        }
        let mut padding = [0u8; 8192];
        loop {
            match self.read(&mut padding) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    if padding.iter().take(n).any(|b| *b != 0) {
                        return Err(AdsbError::Parse(
                            "tar has nonzero data after its end marker".to_owned(),
                        ));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    return Err(AdsbError::Parse(format!("tar transport completion: {e}")));
                }
            }
        }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.saw_eof |= n == 0 && !buf.is_empty();
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Walk a tar stream of `trace_full_*` members and return the retained tracks.
pub fn ingest_reader<R: Read>(
    r: R,
    spec: &DumpSpec,
) -> Result<(Vec<FlightTrack>, DumpStats), AdsbError> {
    let count = Arc::new(AtomicU64::new(0));
    let reader = CountingReader {
        inner: r,
        count: count.clone(),
        saw_eof: false,
    };
    let mut archive = tar::Archive::new(reader);
    let mut aircraft: BTreeMap<String, DumpAircraft> = BTreeMap::new();
    let mut st = DumpStats::default();
    let entries = archive
        .entries()
        .map_err(|e| AdsbError::Parse(format!("tar: {e}")))?;
    for entry in entries {
        let mut e = entry.map_err(|e| AdsbError::Parse(format!("tar entry: {e}")))?;
        if !e.header().entry_type().is_file() {
            continue;
        }
        let name = e
            .path()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !name.contains("trace_full_") {
            continue;
        }
        let mut buf = Vec::with_capacity(e.size() as usize);
        e.read_to_end(&mut buf)
            .map_err(|err| AdsbError::Parse(format!("tar read {name}: {err}")))?;
        st.members += 1;
        accumulate(parse_member(&buf, &name, spec), &mut aircraft, &mut st);
    }
    archive.into_inner().finish_tar()?;
    st.bytes = count.load(Ordering::Relaxed);

    let tracks = aircraft
        .into_iter()
        .filter_map(|(icao24, a)| aircraft_to_track(icao24, a.type_code, a.points))
        .collect();
    Ok((tracks, st))
}

/// Ingest a local source: a `.tar` (release parts must be concatenated
/// first), a directory tree of `trace_full_*` files, or a single
/// `trace_full_*` file (gzip or plain JSON).
pub fn ingest_path(
    path: &Path,
    spec: &DumpSpec,
) -> Result<(Vec<FlightTrack>, DumpStats), AdsbError> {
    if path.is_dir() {
        let mut files = Vec::new();
        collect_trace_files(path, &mut files)?;
        files.sort();
        let mut aircraft: BTreeMap<String, DumpAircraft> = BTreeMap::new();
        let mut st = DumpStats::default();
        for file in files {
            let bytes = std::fs::read(&file)
                .map_err(|e| AdsbError::Parse(format!("{}: {e}", file.display())))?;
            st.members += 1;
            st.bytes += bytes.len() as u64;
            accumulate(
                parse_member(&bytes, &file.to_string_lossy(), spec),
                &mut aircraft,
                &mut st,
            );
        }
        let tracks = aircraft
            .into_iter()
            .filter_map(|(icao24, a)| aircraft_to_track(icao24, a.type_code, a.points))
            .collect();
        return Ok((tracks, st));
    }
    let mut f = std::fs::File::open(path)
        .map_err(|e| AdsbError::Parse(format!("{}: {e}", path.display())))?;
    let mut head = [0u8; 512];
    let n = f
        .read(&mut head)
        .map_err(|e| AdsbError::Parse(format!("{}: {e}", path.display())))?;
    let is_tar = n >= 262 && &head[257..262] == b"ustar";
    drop(f);
    if is_tar {
        let f = std::fs::File::open(path)
            .map_err(|e| AdsbError::Parse(format!("{}: {e}", path.display())))?;
        ingest_reader(std::io::BufReader::with_capacity(1 << 20, f), spec)
    } else {
        let bytes = std::fs::read(path)
            .map_err(|e| AdsbError::Parse(format!("{}: {e}", path.display())))?;
        let mut aircraft: BTreeMap<String, DumpAircraft> = BTreeMap::new();
        let mut st = DumpStats {
            members: 1,
            bytes: bytes.len() as u64,
            ..Default::default()
        };
        accumulate(
            parse_member(&bytes, &path.to_string_lossy(), spec),
            &mut aircraft,
            &mut st,
        );
        let tracks = aircraft
            .into_iter()
            .filter_map(|(icao24, a)| aircraft_to_track(icao24, a.type_code, a.points))
            .collect();
        Ok((tracks, st))
    }
}

fn collect_trace_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AdsbError> {
    for e in
        std::fs::read_dir(dir).map_err(|e| AdsbError::Parse(format!("{}: {e}", dir.display())))?
    {
        let e = e.map_err(|e| AdsbError::Parse(format!("{}: {e}", dir.display())))?;
        let p = e.path();
        if p.is_dir() {
            collect_trace_files(&p, out)?;
        } else if p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("trace_full_"))
        {
            out.push(p);
        }
    }
    Ok(())
}

/// A local adsb.lol archive — a tar, a directory of `trace_full_*` files or
/// a single member — replayed through [`TrackSource`]. Release discovery and
/// download belong to the fetch layer; this source reads what is already on
/// disk, which keeps tests and offline runs free of the network.
#[derive(Debug, Clone)]
pub struct AdsbLolArchive {
    path: PathBuf,
}

impl AdsbLolArchive {
    /// Open an archive at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl TrackSource for AdsbLolArchive {
    fn tracks(&self, query: &RegionQuery) -> Result<Vec<FlightTrack>, AdsbError> {
        query.validate()?;
        let (tracks, _stats) = ingest_path(&self.path, &DumpSpec::from_query(query))?;
        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    const T0: f64 = 1_788_825_600.0; // 2026-09-08T00:00:00Z

    fn spec() -> DumpSpec {
        DumpSpec {
            min_lat_deg: 40.0,
            min_lon_deg: -74.0,
            max_lat_deg: 41.0,
            max_lon_deg: -73.0,
            t_start_s: T0 + 70_000.0,
            t_end_s: T0 + 80_000.0,
        }
    }

    fn gz(s: &str) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::fast());
        e.write_all(s.as_bytes()).expect("gzip write");
        e.finish().expect("gzip finish")
    }

    fn tar_of(members: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, data) in members {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            b.append_data(&mut h, name, data.as_slice())
                .expect("tar append");
        }
        b.into_inner().expect("tar finish")
    }

    /// PC-12: rows 0-1 outside bbox (west), row 2 "ground", rows 3-9 inside, a
    /// new-leg flag on row 5. Geom on 3,4,6 (plus baro), baro-only on 7,8,9.
    fn pc12_json() -> String {
        let rows = [
            r#"[70000.0, 40.5, -74.5, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]"#,
            r#"[70010.0, 40.5, -74.2, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]"#,
            r#"[70020.0, 40.5, -74.1, "ground", 0, 90, 0, null, null, "adsb_icao", null, null, null, null]"#,
            r#"[70030.0, 40.5, -73.9, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]"#,
            r#"[70040.0, 40.5, -73.8, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]"#,
            r#"[70050.0, 40.5, -73.7, 5000, 200, 90, 2, 0, null, "adsb_icao", 5100, 0, null, null]"#,
            r#"[70060.0, 40.5, -73.6, 5000, 200, 90, 8, 0, null, "adsb_icao", null, 0, null, null]"#,
            r#"[70070.0, 40.5, -73.5, 5000, 200, 90, 0, 0, null, "adsb_icao", null, null, null, null]"#,
            r#"[70080.0, 40.5, -73.4, 5000, 200, 90, 0, 0, null, "adsb_icao", null, null, null, null]"#,
            r#"[70090.0, 40.5, -73.3, 5000, 200, 90, 0, 0, null, "adsb_icao", null, null, null, null]"#,
        ];
        format!(
            r#"{{"icao":"a1b2c3","r":"N123AB","t":"PC12","timestamp":{T0},"trace":[{}]}}"#,
            rows.join(",")
        )
    }

    #[test]
    fn parses_a_trace_member_into_a_track() {
        let bytes = tar_of(&[("trace_full_a1b2c3.json", gz(&pc12_json()))]);
        let (tracks, stats) = ingest_reader(bytes.as_slice(), &spec()).expect("ingest");
        assert_eq!(tracks.len(), 1);
        assert_eq!(stats.members, 1);
        assert_eq!(stats.parse_errors, 0);
        let track = &tracks[0];
        assert_eq!(track.icao24, "a1b2c3");
        assert_eq!(track.source, TrackSourceKind::AdsbLol);
        assert_eq!(track.type_code.as_deref(), Some("PC12"));
        assert!(track.callsign.is_none());
        // Rows outside the window keep two neighbours for interpolation, so
        // the first retained sample is the western outlier at dt=70000, not
        // the first in-box row; the ground row drops.
        assert_eq!(track.samples.len(), 9);
        let first = track.samples.first().expect("first sample");
        assert_eq!(first.utc.0, ((T0 + 70_000.0) * 1000.0) as i64);
        assert!((first.geometric_altitude_m - 5100.0 * FT_TO_M).abs() < 1e-6);
        assert!((first.barometric_altitude_m.expect("baro") - 5000.0 * FT_TO_M).abs() < 1e-6);
        assert!((first.ground_speed_m_per_s.expect("gs") - 200.0 * KT_TO_MPS).abs() < 1e-6);
    }

    #[test]
    fn maps_readsb_flags_and_units() {
        let rows = [
            r#"[70030.0, 40.5, -73.9, 5000, 200, 90, 1, 120, null, "mlat", null, 60, null, null]"#,
            r#"[70040.0, 40.5, -73.8, "ground", 0, 90, 2, null, null, "adsb_icao", null, null, null, null]"#,
            r#"[70050.0, 40.5, -73.7, 5000, 200, 90, 0, 0, null, "tisb", null, null, null, null]"#,
        ];
        let json = format!(
            r#"{{"icao":"a1b2c3","t":"PC12","timestamp":{T0},"trace":[{}]}}"#,
            rows.join(",")
        );
        let bytes = tar_of(&[("trace_full_a1b2c3.json", gz(&json))]);
        let (tracks, _) = ingest_reader(bytes.as_slice(), &spec()).expect("ingest");
        let samples = &tracks[0].samples;
        // Row 0: stale + mlat source, geometric vrate preferred over baro vrate.
        let first = samples.first().expect("first");
        assert_eq!(first.flags & FLAG_MLAT, FLAG_MLAT);
        assert_eq!(first.flags & FLAG_STALE, FLAG_STALE);
        assert!((first.vertical_rate_m_per_s.expect("vrate") - 60.0 * FPM_TO_MPS).abs() < 1e-9);
        // Row 2: tisb source, follows a new-leg boundary.
        let last = samples.last().expect("last");
        assert_eq!(last.flags & FLAG_TISB, FLAG_TISB);
        assert_eq!(last.flags & FLAG_NEW_LEG, FLAG_NEW_LEG);
    }

    #[test]
    fn skips_members_without_overlap_and_bad_payloads() {
        let far = r#"{"icao":"fff000","t":"B738","timestamp":1788825600,"trace":[[70030.0, 51.5, -0.1, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]]}"#;
        let bad = b"not json".to_vec();
        let bytes = tar_of(&[
            ("trace_full_fff000.json", gz(far)),
            ("trace_full_bad.json", bad),
        ]);
        let (_tracks, stats) = ingest_reader(bytes.as_slice(), &spec()).expect("ingest");
        assert_eq!(stats.tracks_kept, 0);
        assert_eq!(stats.members, 2);
        assert_eq!(stats.parse_errors, 1);
    }

    #[test]
    fn plain_json_members_work_without_gzip() {
        let bytes = tar_of(&[("trace_full_a1b2c3.json", pc12_json().into_bytes())]);
        let (tracks, stats) = ingest_reader(bytes.as_slice(), &spec()).expect("ingest");
        assert_eq!(tracks.len(), 1);
        assert_eq!(stats.parse_errors, 0);
    }

    #[test]
    fn tar_requires_both_end_blocks_after_the_last_member() {
        let bytes = tar_of(&[("trace_full_a1b2c3.json", gz(&pc12_json()))]);
        let payload_end = bytes.len() - 1024;
        for tail in [0, 1, 511, 512, 513, 1023] {
            let result = ingest_reader(&bytes[..payload_end + tail], &spec());
            assert!(result.is_err(), "accepted only {tail} end-marker bytes");
        }
        let mut corrupt = bytes;
        let second_block = corrupt.len() - 512;
        corrupt[second_block] = 1;
        assert!(ingest_reader(corrupt.as_slice(), &spec()).is_err());
    }

    #[test]
    fn tar_rejects_nonzero_data_after_the_end_marker() {
        let mut bytes = tar_of(&[("trace_full_a1b2c3.json", gz(&pc12_json()))]);
        bytes.extend_from_slice(b"unexpected second archive or damaged padding");
        assert!(ingest_reader(bytes.as_slice(), &spec()).is_err());
    }

    #[test]
    fn tar_drains_zero_padding_and_keeps_split_member_boundaries() {
        struct Chunks<R>(R, usize);
        impl<R: Read> Read for Chunks<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = buf.len().min(self.1);
                self.0.read(&mut buf[..n])
            }
        }
        let mut bytes = tar_of(&[("trace_full_a1b2c3.json", gz(&pc12_json()))]);
        bytes.resize(bytes.len() + 9 * 512, 0); // normal tar blocking-factor padding
        for split in [1, 17, 511, 512, 1025] {
            let (head, tail) = bytes.split_at(split);
            let reader = head.chain(tail);
            let (tracks, stats) = ingest_reader(Chunks(reader, 37), &spec()).expect("ingest");
            assert_eq!(tracks.len(), 1);
            assert_eq!(stats.members, 1);
            assert_eq!(stats.bytes, bytes.len() as u64);
        }
    }

    #[test]
    fn timed_segments_keep_interpolation_anchors() {
        // Neither endpoint is inside the window, but the segment crosses it
        // inside the box, so both endpoints are kept.
        let rows = [
            r#"[60000.0, 40.5, -74.5, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]"#,
            r#"[90000.0, 40.5, -73.5, 5000, 200, 90, 0, 0, null, "adsb_icao", 5100, 0, null, null]"#,
        ];
        let json = format!(
            r#"{{"icao":"a1b2c3","t":"PC12","timestamp":{T0},"trace":[{}]}}"#,
            rows.join(",")
        );
        let bytes = tar_of(&[("trace_full_a1b2c3.json", gz(&json))]);
        let (tracks, _) = ingest_reader(bytes.as_slice(), &spec()).expect("ingest");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].samples.len(), 2);
    }

    #[test]
    fn adsb_lol_archive_replays_a_local_tar_through_track_source() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("day.tar");
        std::fs::write(
            &path,
            tar_of(&[("trace_full_a1b2c3.json", gz(&pc12_json()))]),
        )
        .expect("write tar");
        let source = AdsbLolArchive::new(&path);
        let query = RegionQuery {
            min_lat_deg: 40.0,
            min_lon_deg: -74.0,
            max_lat_deg: 41.0,
            max_lon_deg: -73.0,
            from: UtcMillis(((T0 + 70_000.0) * 1000.0) as i64),
            to: UtcMillis(((T0 + 80_000.0) * 1000.0) as i64),
        };
        let tracks = source.tracks(&query).expect("tracks");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].icao24, "a1b2c3");
    }

    #[test]
    fn spec_segment_intersection_takes_the_short_way_round_the_dateline() {
        // A box on the far side of the antimeridian: the segment from 175°E
        // to 175°W crosses it inside the window.
        let spec = DumpSpec {
            min_lat_deg: -10.0,
            min_lon_deg: -180.0,
            max_lat_deg: 10.0,
            max_lon_deg: -170.0,
            t_start_s: 0.0,
            t_end_s: 10.0,
        };
        assert!(spec.intersects_timed_segment((0.0, 0.0, 175.0), (5.0, 0.0, -175.0)));
        // Outside the time window.
        assert!(!spec.intersects_timed_segment((20.0, 0.0, 175.0), (25.0, 0.0, -175.0)));
        // A box nowhere near the path does not intersect.
        let far = DumpSpec {
            min_lon_deg: 0.0,
            max_lon_deg: 10.0,
            ..spec
        };
        assert!(!far.intersects_timed_segment((0.0, 0.0, 175.0), (5.0, 0.0, -175.0)));
    }
}

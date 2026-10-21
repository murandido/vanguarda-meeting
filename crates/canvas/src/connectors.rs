//! Connectors (CA-03): arrows between shapes.
//!
//! A connector is stored in the same `shapes` map as every other shape, so it
//! shares ids, deletion and the board size limits:
//!
//! ```text
//! shapes[id] = {
//!   kind:  "connector",
//!   from:  { shape: "<id>", side: "n" | "e" | "s" | "w" | "auto" }  // or { x, y }
//!   to:    { shape: "<id>", side: ... }                             // or { x, y }
//!   route: "curved" | "straight"                                    // optional, default curved
//! }
//! ```
//!
//! `from`/`to` are plain (atomic) Loro map values: an endpoint is replaced as a
//! whole, never merged field by field. A shape endpoint references any shape
//! that has bounds (`x`, `y`, `w`, `h`), so it works for every shape kind.
//!
//! Geometry is never stored: it is recomputed from the current bounds of the
//! referenced shapes, so an arrow follows its shapes when they move, locally
//! or remotely. A connector whose shape no longer exists (e.g. deleted by
//! another participant at the same time the connector was created) is
//! *dangling*: it stays in the document but is neither drawn nor hit-tested.
//! Deleting a shape locally also deletes the connectors attached to it (see
//! [`Board::remove_shape`]).

use std::cell::RefCell;
use std::collections::HashMap;

use loro::{Frontiers, LoroMap, LoroValue, ValueOrContainer};

use crate::{Board, Bounds, Shape};

/// Value of the `kind` field that marks a connector.
pub const KIND: &str = "connector";

/// How far the control points of a curved connector stick out of the shape,
/// as a fraction of the distance between the endpoints.
const CURVE_PULL: f64 = 0.5;
/// Minimum and maximum control point distance, in world units.
const CURVE_MIN: f64 = 24.0;
const CURVE_MAX: f64 = 240.0;
/// Line segments used to approximate a curve when hit-testing.
const HIT_SEGMENTS: u32 = 32;

/// Number of `f64` per connector in the flat geometry arrays: the four cubic
/// Bézier points followed by the unit direction of the arrowhead.
pub const STRIDE: usize = 10;

pub type Point = (f64, f64);

/// Side of a shape's bounding box an endpoint is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    N,
    E,
    S,
    W,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::N, Side::E, Side::S, Side::W];

    /// Parses `"n" | "e" | "s" | "w"`; `"auto"` (or anything else) is `None`,
    /// meaning "the side facing the other end".
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "n" => Some(Side::N),
            "e" => Some(Side::E),
            "s" => Some(Side::S),
            "w" => Some(Side::W),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(side: Option<Self>) -> &'static str {
        match side {
            Some(Side::N) => "n",
            Some(Side::E) => "e",
            Some(Side::S) => "s",
            Some(Side::W) => "w",
            None => "auto",
        }
    }

    /// Outward unit normal (y grows downwards).
    #[must_use]
    pub fn normal(self) -> Point {
        match self {
            Side::N => (0.0, -1.0),
            Side::E => (1.0, 0.0),
            Side::S => (0.0, 1.0),
            Side::W => (-1.0, 0.0),
        }
    }

    /// Midpoint of this side of `b`, where arrows attach.
    #[must_use]
    pub fn anchor(self, b: &Bounds) -> Point {
        let (cx, cy) = center(b);
        match self {
            Side::N => (cx, b.min_y),
            Side::E => (b.max_x, cy),
            Side::S => (cx, b.max_y),
            Side::W => (b.min_x, cy),
        }
    }

    /// Side of `b` that faces `target`, weighting each axis by the box size so
    /// wide shapes prefer their long sides.
    #[must_use]
    pub fn facing(b: &Bounds, target: Point) -> Self {
        let (cx, cy) = center(b);
        let dx = (target.0 - cx) / b.width().max(1.0);
        let dy = (target.1 - cy) / b.height().max(1.0);
        if dx.abs() >= dy.abs() {
            if dx >= 0.0 { Side::E } else { Side::W }
        } else if dy >= 0.0 {
            Side::S
        } else {
            Side::N
        }
    }
}

/// One end of a connector.
#[derive(Debug, Clone, PartialEq)]
pub enum Endpoint {
    /// Attached to a shape; `side: None` picks the side facing the other end.
    Shape { shape: String, side: Option<Side> },
    /// A free point in world coordinates.
    Point { x: f64, y: f64 },
}

impl Endpoint {
    #[must_use]
    pub fn shape(id: &str, side: Option<Side>) -> Self {
        Endpoint::Shape {
            shape: id.to_owned(),
            side,
        }
    }

    fn shape_id(&self) -> Option<&str> {
        match self {
            Endpoint::Shape { shape, .. } => Some(shape),
            Endpoint::Point { .. } => None,
        }
    }

    fn to_value(&self) -> LoroValue {
        let mut m: HashMap<String, LoroValue> = HashMap::new();
        match self {
            Endpoint::Shape { shape, side } => {
                m.insert("shape".into(), shape.as_str().into());
                m.insert("side".into(), Side::as_str(*side).into());
            }
            Endpoint::Point { x, y } => {
                m.insert("x".into(), (*x).into());
                m.insert("y".into(), (*y).into());
            }
        }
        m.into()
    }

    fn from_value(v: &LoroValue) -> Option<Self> {
        let LoroValue::Map(m) = v else { return None };
        if let Some(LoroValue::String(shape)) = m.get("shape") {
            let side = match m.get("side") {
                Some(LoroValue::String(s)) => Side::parse(s),
                _ => None,
            };
            return Some(Endpoint::shape(shape, side));
        }
        let num = |k: &str| match m.get(k) {
            Some(LoroValue::Double(n)) if n.is_finite() => Some(*n),
            _ => None,
        };
        Some(Endpoint::Point {
            x: num("x")?,
            y: num("y")?,
        })
    }
}

/// Line style between the endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Route {
    #[default]
    Curved,
    Straight,
}

impl Route {
    fn as_str(self) -> &'static str {
        match self {
            Route::Curved => "curved",
            Route::Straight => "straight",
        }
    }
}

/// A connector as read from the document.
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    pub id: String,
    pub from: Endpoint,
    pub to: Endpoint,
    pub route: Route,
}

/// Cubic Bézier `p0 → p3` with control points `c1`, `c2`. Straight lines are
/// cubics too (controls on the segment), so drawing and hit-testing share one
/// code path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    pub p0: Point,
    pub c1: Point,
    pub c2: Point,
    pub p3: Point,
}

impl Curve {
    #[must_use]
    pub fn point(&self, t: f64) -> Point {
        let s = 1.0 - t;
        // Bernstein weights of the four points.
        let w = [s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t];
        let pts = [self.p0, self.c1, self.c2, self.p3];
        pts.iter()
            .zip(w)
            .fold((0.0, 0.0), |acc, (p, w)| (acc.0 + w * p.0, acc.1 + w * p.1))
    }

    /// Box containing the whole curve (the convex hull of the control points
    /// contains a Bézier curve).
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        let pts = [self.p0, self.c1, self.c2, self.p3];
        let mut b = Bounds {
            min_x: f64::INFINITY,
            min_y: f64::INFINITY,
            max_x: f64::NEG_INFINITY,
            max_y: f64::NEG_INFINITY,
        };
        for (x, y) in pts {
            b.min_x = b.min_x.min(x);
            b.min_y = b.min_y.min(y);
            b.max_x = b.max_x.max(x);
            b.max_y = b.max_y.max(y);
        }
        b
    }

    /// Approximate distance from `p` to the curve (polyline of
    /// [`HIT_SEGMENTS`] segments — far below a pixel for on-screen arrows).
    #[must_use]
    pub fn distance(&self, p: Point) -> f64 {
        let mut best = f64::INFINITY;
        let mut prev = self.p0;
        for i in 1..=HIT_SEGMENTS {
            let next = self.point(f64::from(i) / f64::from(HIT_SEGMENTS));
            best = best.min(segment_distance(p, prev, next));
            prev = next;
        }
        best
    }

    /// Unit direction the curve arrives at `p3` with, for the arrowhead. Falls
    /// back to earlier control points when the curve is degenerate there.
    #[must_use]
    pub fn end_direction(&self) -> Point {
        [self.c2, self.c1, self.p0]
            .into_iter()
            .find_map(|from| unit(sub(self.p3, from)))
            .unwrap_or((1.0, 0.0))
    }

    fn flat(&self) -> [f64; STRIDE] {
        let (dx, dy) = self.end_direction();
        [
            self.p0.0, self.p0.1, self.c1.0, self.c1.1, self.c2.0, self.c2.1, self.p3.0, self.p3.1,
            dx, dy,
        ]
    }
}

/// Builds the curve between two resolved ends. Each end is a point plus the
/// outward normal of its side, if attached to a shape.
#[must_use]
pub fn route(from: (Point, Option<Point>), to: (Point, Option<Point>), style: Route) -> Curve {
    let (p0, n0) = from;
    let (p3, n3) = to;
    if style == Route::Straight || (n0.is_none() && n3.is_none()) {
        return Curve {
            p0,
            c1: lerp(p0, p3, 1.0 / 3.0),
            c2: lerp(p0, p3, 2.0 / 3.0),
            p3,
        };
    }
    let pull = (dist(p0, p3) * CURVE_PULL).clamp(CURVE_MIN, CURVE_MAX);
    let out = |p: Point, n: Point| (p.0 + n.0 * pull, p.1 + n.1 * pull);
    // A free end has no normal: its control point sits a third of the way
    // towards the other end's control point, so the curve still arrives
    // smoothly instead of with a kink.
    let (c1, c2) = match (n0, n3) {
        (Some(n0), Some(n3)) => (out(p0, n0), out(p3, n3)),
        (Some(n0), None) => {
            let c1 = out(p0, n0);
            (c1, lerp(p3, c1, 1.0 / 3.0))
        }
        (None, Some(n3)) => {
            let c2 = out(p3, n3);
            (lerp(p0, c2, 1.0 / 3.0), c2)
        }
        (None, None) => unreachable!("handled above"),
    };
    Curve { p0, c1, c2, p3 }
}

/// Resolves both ends against the current shape bounds. `None` if an end
/// references a shape that does not exist (a dangling connector).
#[must_use]
pub fn resolve(
    from: &Endpoint,
    to: &Endpoint,
    style: Route,
    bounds_of: impl Fn(&str) -> Option<Bounds>,
) -> Option<Curve> {
    // Every end needs a reference point on the other end to pick an "auto"
    // side: the other shape's center, or the free point itself.
    let reference = |e: &Endpoint| -> Option<(Point, Option<Bounds>)> {
        match e {
            Endpoint::Point { x, y } => Some(((*x, *y), None)),
            Endpoint::Shape { shape, .. } => {
                let b = bounds_of(shape)?;
                Some((center(&b), Some(b)))
            }
        }
    };
    let (ref_from, b_from) = reference(from)?;
    let (ref_to, b_to) = reference(to)?;
    let end = |e: &Endpoint, b: Option<Bounds>, other: Point| match (e, b) {
        (Endpoint::Shape { side, .. }, Some(b)) => {
            let side = side.unwrap_or_else(|| Side::facing(&b, other));
            (side.anchor(&b), Some(side.normal()))
        }
        _ => (ref_point(e), None),
    };
    Some(route(
        end(from, b_from, ref_to),
        end(to, b_to, ref_from),
        style,
    ))
}

fn ref_point(e: &Endpoint) -> Point {
    match e {
        Endpoint::Point { x, y } => (*x, *y),
        Endpoint::Shape { .. } => (0.0, 0.0),
    }
}

/// A connector with its geometry resolved against the current shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct Routed {
    pub id: String,
    pub curve: Curve,
}

/// Connectors read from the document plus their resolved geometry, valid
/// while the document frontiers do not change.
#[derive(Default)]
pub struct Cache(RefCell<Option<CacheEntry>>);

/// Frontiers the entry was read at, all connectors, the routed ones.
type CacheEntry = (Frontiers, Vec<Connector>, Vec<Routed>);

impl Board {
    /// Creates (or replaces) connector `id`.
    pub fn add_connector(&self, id: &str, from: &Endpoint, to: &Endpoint, route: Route) {
        let map = self
            .shape_map()
            .insert_container(id, LoroMap::new())
            .expect("valid map key");
        map.insert("kind", KIND).expect("scalar value");
        map.insert("from", from.to_value()).expect("map value");
        map.insert("to", to.to_value()).expect("map value");
        map.insert("route", route.as_str()).expect("scalar value");
        self.doc.commit();
    }

    /// Changes the line style of connector `id`; returns `false` if there is
    /// no such connector.
    pub fn set_connector_route(&self, id: &str, route: Route) -> bool {
        let Some(map) = self.connector_map(id) else {
            return false;
        };
        map.insert("route", route.as_str()).expect("scalar value");
        self.doc.commit();
        true
    }

    fn connector_map(&self, id: &str) -> Option<LoroMap> {
        let ValueOrContainer::Container(c) = self.shape_map().get(id)? else {
            return None;
        };
        let map = c.into_map().ok()?;
        is_connector(&map).then_some(map)
    }

    /// All connectors in the document, dangling ones included, sorted by id.
    #[must_use]
    pub fn connectors(&self) -> Vec<Connector> {
        self.with_connectors(|all, _| all.to_vec())
    }

    /// Connectors whose ends all resolve, with their geometry, in draw order.
    #[must_use]
    pub fn routed_connectors(&self) -> Vec<Routed> {
        self.with_connectors(|_, routed| routed.to_vec())
    }

    fn with_connectors<R>(&self, f: impl FnOnce(&[Connector], &[Routed]) -> R) -> R {
        let now = self.doc.state_frontiers();
        let mut cache = self.connector_cache.0.borrow_mut();
        if cache.as_ref().is_none_or(|(at, _, _)| *at != now) {
            let all = self.read_connectors();
            let routed = if all.is_empty() {
                Vec::new()
            } else {
                self.with_shapes(|shapes| route_all(&all, shapes))
            };
            *cache = Some((now, all, routed));
        }
        let (_, all, routed) = cache.as_ref().expect("cache filled above");
        f(all, routed)
    }

    fn read_connectors(&self) -> Vec<Connector> {
        let mut out = Vec::new();
        self.shape_map().for_each(|id, value| {
            let ValueOrContainer::Container(c) = value else {
                return;
            };
            let Ok(map) = c.into_map() else { return };
            if !is_connector(&map) {
                return;
            }
            let end = |k: &str| match map.get(k) {
                Some(ValueOrContainer::Value(v)) => Endpoint::from_value(&v),
                _ => None,
            };
            let route = match map.get("route") {
                Some(ValueOrContainer::Value(LoroValue::String(s))) if s.as_str() == "straight" => {
                    Route::Straight
                }
                _ => Route::Curved,
            };
            if let (Some(from), Some(to)) = (end("from"), end("to")) {
                out.push(Connector {
                    id: id.to_owned(),
                    from,
                    to,
                    route,
                });
            }
        });
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Ids of connectors with an end attached to shape `id`.
    pub(crate) fn connectors_attached_to(&self, id: &str) -> Vec<String> {
        self.with_connectors(|all, _| {
            all.iter()
                .filter(|c| c.from.shape_id() == Some(id) || c.to.shape_id() == Some(id))
                .map(|c| c.id.clone())
                .collect()
        })
    }

    /// Flat geometry ([`STRIDE`] numbers each) of the connectors that touch
    /// `view` inflated by `margin` (room for the arrowhead), in draw order.
    #[must_use]
    pub fn visible_connectors(&self, view: &Bounds, margin: f64) -> Vec<f64> {
        let view = inflate(view, margin);
        self.with_connectors(|_, routed| {
            routed
                .iter()
                .filter(|r| r.curve.bounds().intersects(&view))
                .flat_map(|r| r.curve.flat())
                .collect()
        })
    }

    /// Flat geometry of one connector, empty if missing or dangling.
    #[must_use]
    pub fn connector_geometry(&self, id: &str) -> Vec<f64> {
        self.with_connectors(|_, routed| {
            routed
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.curve.flat().to_vec())
                .unwrap_or_default()
        })
    }

    /// Topmost connector passing within `tolerance` (world units) of `(x, y)`.
    #[must_use]
    pub fn hit_connector(&self, x: f64, y: f64, tolerance: f64) -> Option<String> {
        self.with_connectors(|_, routed| {
            routed
                .iter()
                .rev()
                .find(|r| {
                    inflate(&r.curve.bounds(), tolerance).contains(x, y)
                        && r.curve.distance((x, y)) <= tolerance
                })
                .map(|r| r.id.clone())
        })
    }

    /// Geometry of a connector being drawn (not yet in the document).
    #[must_use]
    pub fn preview_connector(&self, from: &Endpoint, to: &Endpoint, style: Route) -> Vec<f64> {
        self.with_shapes(|shapes| resolve(from, to, style, bounds_lookup(shapes)))
            .map(|c| c.flat().to_vec())
            .unwrap_or_default()
    }

    /// Anchor points `[nx, ny, ex, ey, sx, sy, wx, wy]` of shape `id`, or
    /// empty if it does not exist.
    #[must_use]
    pub fn anchor_points(&self, id: &str) -> Vec<f64> {
        self.with_shapes(|shapes| bounds_lookup(shapes)(id))
            .map(|b| {
                Side::ALL
                    .iter()
                    .flat_map(|s| {
                        let (x, y) = s.anchor(&b);
                        [x, y]
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Anchor within `tolerance` of `(x, y)`, preferring the topmost shape.
    #[must_use]
    pub fn anchor_near(&self, x: f64, y: f64, tolerance: f64) -> Option<(String, Side)> {
        self.with_shapes(|shapes| {
            shapes.iter().rev().find_map(|r| {
                let b = r.bounds();
                Side::ALL
                    .into_iter()
                    .find(|s| dist(s.anchor(&b), (x, y)) <= tolerance)
                    .map(|s| (r.id.clone(), s))
            })
        })
    }
}

fn is_connector(map: &LoroMap) -> bool {
    matches!(
        map.get("kind"),
        Some(ValueOrContainer::Value(LoroValue::String(k))) if k.as_str() == KIND
    )
}

/// Bounds of shape `id`, for one-off lookups.
fn bounds_lookup(shapes: &[Shape]) -> impl Fn(&str) -> Option<Bounds> + '_ {
    move |id| shapes.iter().find(|r| r.id == id).map(Shape::bounds)
}

/// Routes every connector, indexing the shapes once so this stays linear in
/// shapes + connectors whatever order the shapes are drawn in.
fn route_all(all: &[Connector], shapes: &[Shape]) -> Vec<Routed> {
    let index: HashMap<&str, Bounds> = shapes.iter().map(|r| (r.id.as_str(), r.bounds())).collect();
    all.iter()
        .filter_map(|c| {
            resolve(&c.from, &c.to, c.route, |id| index.get(id).copied()).map(|curve| Routed {
                id: c.id.clone(),
                curve,
            })
        })
        .collect()
}

fn center(b: &Bounds) -> Point {
    (
        f64::midpoint(b.min_x, b.max_x),
        f64::midpoint(b.min_y, b.max_y),
    )
}

fn inflate(b: &Bounds, m: f64) -> Bounds {
    Bounds {
        min_x: b.min_x - m,
        min_y: b.min_y - m,
        max_x: b.max_x + m,
        max_y: b.max_y + m,
    }
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn dist(a: Point, b: Point) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

fn unit(v: Point) -> Option<Point> {
    let len = v.0.hypot(v.1);
    (len > 1e-9).then(|| (v.0 / len, v.1 / len))
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = sub(b, a);
    let len2 = ab.0 * ab.0 + ab.1 * ab.1;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let ap = sub(p, a);
    let t = ((ap.0 * ab.0 + ap.1 * ab.1) / len2).clamp(0.0, 1.0);
    dist(p, lerp(a, b, t))
}

#[cfg(test)]
mod tests;

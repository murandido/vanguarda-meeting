//! Cartazes engine: board state in a Loro document (CRDT).
//!
//! A board holds post-its (CA-02) and stickers (CA-04); see [`shape`] for the
//! document schema. The camera (pan/zoom, local to each participant) lives in
//! [`camera`]; the browser bridge lives in [`wasm`], compiled only for `wasm32`.

use std::cell::RefCell;

use loro::{ExportMode, Frontiers, LoroDoc, LoroMap, LoroValue, VersionVector};

pub mod camera;
pub mod connectors;
pub mod shape;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

mod limits;
pub use camera::{Bounds, Camera};
pub use limits::{LimitError, Limits};
pub use shape::{Body, Color, RECORD, STICKERS, Shape};

const SHAPES: &str = "shapes";

const KIND_POSTIT: &str = "postit";
const KIND_STICKER: &str = "sticker";

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("atualização ou snapshot inválido: {0}")]
    Import(String),
    #[error("falha ao exportar o documento: {0}")]
    Export(String),
    #[error("forma inexistente: {0}")]
    UnknownShape(String),
    #[error("operação não se aplica a esta forma: {0}")]
    WrongKind(String),
    #[error("cor desconhecida: {0}")]
    UnknownColor(String),
    #[error("figurinha desconhecida: {0}")]
    UnknownSticker(String),
    #[error("valor numérico inválido")]
    NotFinite,
}

/// Shapes that touch `view`, in the draw order of `shapes`.
pub fn cull<'a>(shapes: &'a [Shape], view: &'a Bounds) -> impl Iterator<Item = &'a Shape> + 'a {
    shapes.iter().filter(move |s| s.bounds().intersects(view))
}

fn finite(values: &[f64]) -> Result<(), BoardError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(BoardError::NotFinite)
    }
}

/// A board: the Loro document plus how far local changes have been sent.
pub struct Board {
    doc: LoroDoc,
    sent: VersionVector,
    /// Shapes already read from the document, valid while the state
    /// frontiers stay the same. Re-reading the whole document every frame
    /// would be costly with thousands of shapes; comparing frontiers is cheap.
    cache: RefCell<Option<(Frontiers, Vec<Shape>)>>,
    /// Connectors and their routed geometry (see [`connectors`]).
    connector_cache: connectors::Cache,
}

impl Default for Board {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    #[must_use]
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        let sent = doc.oplog_vv();
        Self {
            doc,
            sent,
            cache: RefCell::new(None),
            connector_cache: connectors::Cache::default(),
        }
    }

    /// Rebuilds a board from a snapshot stored on the server.
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, BoardError> {
        let mut board = Self::new();
        board.import(bytes)?;
        board.sent = board.doc.oplog_vv();
        Ok(board)
    }

    /// Pins the peer id; only needed by tests that require determinism.
    pub fn set_peer_id(&self, peer: u64) {
        self.doc
            .set_peer_id(peer)
            .expect("peer id set before any edit");
    }

    fn shape_map(&self) -> LoroMap {
        self.doc.get_map(SHAPES)
    }

    fn new_shape(&self, id: &str, kind: &str, x: f64, y: f64) -> LoroMap {
        let shape = self
            .shape_map()
            .insert_container(id, LoroMap::new())
            .expect("valid map key");
        shape.insert("kind", kind).expect("scalar value");
        shape.insert("x", x).expect("scalar value");
        shape.insert("y", y).expect("scalar value");
        shape
    }

    /// Existing shape map for `id`, and whether it is a post-it.
    fn shape_entry(&self, id: &str) -> Result<(LoroMap, bool), BoardError> {
        let shape = self
            .shape_map()
            .get(id)
            .and_then(|v| v.into_container().ok())
            .and_then(|c| c.into_map().ok())
            .ok_or_else(|| BoardError::UnknownShape(id.to_owned()))?;
        let is_postit = match shape.get("kind").and_then(|v| v.into_value().ok()) {
            None => true,
            Some(LoroValue::String(kind)) => kind.as_str() == KIND_POSTIT,
            Some(_) => false,
        };
        Ok((shape, is_postit))
    }

    fn postit_entry(&self, id: &str) -> Result<LoroMap, BoardError> {
        match self.shape_entry(id)? {
            (shape, true) => Ok(shape),
            (_, false) => Err(BoardError::WrongKind(id.to_owned())),
        }
    }

    fn sticker_entry(&self, id: &str) -> Result<LoroMap, BoardError> {
        match self.shape_entry(id)? {
            (shape, false) => Ok(shape),
            (_, true) => Err(BoardError::WrongKind(id.to_owned())),
        }
    }

    /// Creates an empty post-it with its top-left corner at `(x, y)`. The size
    /// is clamped to the allowed range.
    pub fn add_postit(
        &self,
        id: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        color: Color,
    ) -> Result<(), BoardError> {
        finite(&[x, y, w, h])?;
        let shape = self.new_shape(id, KIND_POSTIT, x, y);
        shape
            .insert("w", shape::clamp_size(w))
            .expect("scalar value");
        shape
            .insert("h", shape::clamp_size(h))
            .expect("scalar value");
        shape.insert("color", color.name()).expect("scalar value");
        shape.ensure_mergeable_text("text").expect("valid map key");
        self.doc.commit();
        Ok(())
    }

    /// Writes a Phase 0 rectangle (`x, y, w, h` only). New code creates
    /// post-its with [`Board::add_postit`]; this exists to exercise the legacy
    /// format, which is read as a yellow post-it without text.
    pub fn add_rect(&self, id: &str, x: f64, y: f64, w: f64, h: f64) {
        let shape = self
            .shape_map()
            .insert_container(id, LoroMap::new())
            .expect("valid map key");
        for (key, value) in [("x", x), ("y", y), ("w", w), ("h", h)] {
            shape.insert(key, value).expect("scalar value");
        }
        self.doc.commit();
    }

    /// Stamps `emoji` (one of [`STICKERS`]) centered on `(x, y)`.
    pub fn add_sticker(&self, id: &str, emoji: &str, x: f64, y: f64) -> Result<(), BoardError> {
        finite(&[x, y])?;
        if !STICKERS.contains(&emoji) {
            return Err(BoardError::UnknownSticker(emoji.to_owned()));
        }
        let shape = self.new_shape(id, KIND_STICKER, x, y);
        shape.insert("emoji", emoji).expect("scalar value");
        shape.insert("scale", 1.0).expect("scalar value");
        shape.insert("rotation", 0.0).expect("scalar value");
        self.doc.commit();
        Ok(())
    }

    /// Moves any shape: `(x, y)` is a post-it's corner or a sticker's center.
    pub fn move_shape(&self, id: &str, x: f64, y: f64) -> Result<(), BoardError> {
        finite(&[x, y])?;
        let (shape, _) = self.shape_entry(id)?;
        shape.insert("x", x).expect("scalar value");
        shape.insert("y", y).expect("scalar value");
        self.doc.commit();
        Ok(())
    }

    /// Resizes a post-it, keeping its top-left corner. The size is clamped.
    pub fn resize_postit(&self, id: &str, w: f64, h: f64) -> Result<(), BoardError> {
        finite(&[w, h])?;
        let shape = self.postit_entry(id)?;
        shape
            .insert("w", shape::clamp_size(w))
            .expect("scalar value");
        shape
            .insert("h", shape::clamp_size(h))
            .expect("scalar value");
        self.doc.commit();
        Ok(())
    }

    pub fn set_color(&self, id: &str, color: Color) -> Result<(), BoardError> {
        let shape = self.postit_entry(id)?;
        shape.insert("color", color.name()).expect("scalar value");
        self.doc.commit();
        Ok(())
    }

    /// Replaces a post-it's text with `text` (cut to
    /// [`shape::MAX_TEXT_CHARS`]), recording only the characters that changed
    /// so concurrent edits merge instead of overwriting each other.
    pub fn set_text(&self, id: &str, text: &str) -> Result<(), BoardError> {
        let shape = self.postit_entry(id)?;
        // A mergeable child: if two users type into a legacy rectangle (which
        // has no text yet) at the same time, both end up in the same text
        // instead of one container replacing the other.
        let container = shape
            .ensure_mergeable_text("text")
            .map_err(|_| BoardError::WrongKind(id.to_owned()))?;
        let old = container.to_string();
        if let Some((pos, deleted, inserted)) = shape::text_diff(&old, shape::truncate_text(text)) {
            if deleted > 0 {
                container.delete(pos, deleted).expect("range inside text");
            }
            if !inserted.is_empty() {
                container
                    .insert(pos, inserted)
                    .expect("position inside text");
            }
            self.doc.commit();
        }
        Ok(())
    }

    /// Current text of a post-it (empty for legacy rectangles).
    pub fn text(&self, id: &str) -> Result<String, BoardError> {
        let shape = self.postit_entry(id)?;
        Ok(shape
            .get("text")
            .and_then(|v| v.into_container().ok())
            .and_then(|c| c.into_text().ok())
            .map(|t| t.to_string())
            .unwrap_or_default())
    }

    /// Sets a sticker's scale (clamped) and rotation (radians, normalized).
    pub fn transform_sticker(&self, id: &str, scale: f64, rotation: f64) -> Result<(), BoardError> {
        finite(&[scale, rotation])?;
        let shape = self.sticker_entry(id)?;
        shape
            .insert("scale", shape::clamp_scale(scale))
            .expect("scalar value");
        shape
            .insert("rotation", shape::normalize_angle(rotation))
            .expect("scalar value");
        self.doc.commit();
        Ok(())
    }

    /// Removes a shape together with the connectors attached to it, in one
    /// commit: an arrow pointing at nothing carries no meaning.
    pub fn remove_shape(&self, id: &str) {
        for connector in self.connectors_attached_to(id) {
            self.shape_map().delete(&connector).expect("valid map key");
        }
        self.shape_map().delete(id).expect("valid map key");
        self.doc.commit();
    }

    /// Every shape in draw order (the last one is on top): post-its, then
    /// stickers, each group sorted by id. Clients make ids sortable by
    /// creation time, so newer shapes land on top of older ones.
    #[must_use]
    pub fn shapes(&self) -> Vec<Shape> {
        self.with_shapes(<[Shape]>::to_vec)
    }

    /// Runs `f` over the current shapes, re-reading the document only if it
    /// changed since the last read.
    pub fn with_shapes<R>(&self, f: impl FnOnce(&[Shape]) -> R) -> R {
        let now = self.doc.state_frontiers();
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|(at, _)| *at != now) {
            *cache = Some((now, self.read_shapes()));
        }
        let (_, shapes) = cache.as_ref().expect("cache filled above");
        f(shapes)
    }

    /// Shape with the given id, if it exists and can be drawn.
    #[must_use]
    pub fn shape(&self, id: &str) -> Option<Shape> {
        self.with_shapes(|shapes| shapes.iter().find(|s| s.id == id).cloned())
    }

    /// Text of the shape at `index` in [`Board::shapes`] (empty for stickers
    /// or an index out of range). JS calls this with the index from a record
    /// only when the text version in that record is new to it.
    #[must_use]
    pub fn text_at(&self, index: usize) -> String {
        self.with_shapes(|shapes| match shapes.get(index).map(|s| &s.body) {
            Some(Body::Postit { text, .. }) => text.clone(),
            _ => String::new(),
        })
    }

    /// Records ([`RECORD`] values each) of the shapes visible in `view`, in
    /// draw order.
    #[must_use]
    pub fn visible_records(&self, view: &Bounds) -> Vec<f64> {
        self.with_shapes(|shapes| {
            shapes
                .iter()
                .enumerate()
                .filter(|(_, s)| s.bounds().intersects(view))
                .flat_map(|(i, s)| s.record(i))
                .collect()
        })
    }

    /// Topmost shape (last in draw order) under the world point.
    #[must_use]
    pub fn hit_test(&self, x: f64, y: f64) -> Option<Shape> {
        self.with_shapes(|shapes| shapes.iter().rev().find(|s| s.contains(x, y)).cloned())
    }

    /// Box around all content, or `None` for an empty board.
    #[must_use]
    pub fn content_bounds(&self) -> Option<Bounds> {
        self.with_shapes(|shapes| shapes.iter().map(Shape::bounds).reduce(|a, b| a.union(&b)))
    }

    fn read_shapes(&self) -> Vec<Shape> {
        let LoroValue::Map(entries) = self.shape_map().get_deep_value() else {
            return Vec::new();
        };
        let mut shapes: Vec<Shape> = entries
            .iter()
            .filter_map(|(id, value)| Shape::parse(id, value))
            .collect();
        shapes.sort_by(|a, b| a.layer().cmp(&b.layer()).then_with(|| a.id.cmp(&b.id)));
        shapes
    }

    /// Applies an update or snapshot from another participant.
    pub fn import(&mut self, bytes: &[u8]) -> Result<(), BoardError> {
        self.doc
            .import(bytes)
            .map(|_| ())
            .map_err(|e| BoardError::Import(e.to_string()))
    }

    /// Changes not sent yet, or `None` if there are none.
    pub fn take_pending_update(&mut self) -> Result<Option<Vec<u8>>, BoardError> {
        let now = self.doc.oplog_vv();
        if now == self.sent {
            return Ok(None);
        }
        let update = self
            .doc
            .export(ExportMode::updates(&self.sent))
            .map_err(|e| BoardError::Export(e.to_string()))?;
        self.sent = now;
        Ok(Some(update))
    }

    pub fn snapshot(&self) -> Result<Vec<u8>, BoardError> {
        self.doc
            .export(ExportMode::Snapshot)
            .map_err(|e| BoardError::Export(e.to_string()))
    }
}

#[cfg(test)]
mod tests;

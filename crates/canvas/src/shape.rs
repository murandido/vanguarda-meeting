//! Shapes on a board (CA-02 sticky notes, CA-04 stickers) and how they are
//! stored in the Loro document.
//!
//! # Schema
//!
//! The root map `shapes` maps a shape id to a `LoroMap` with these fields:
//!
//! | key        | type       | kind      | meaning                                         |
//! |------------|------------|-----------|-------------------------------------------------|
//! | `kind`     | string     | all       | `"postit"` or `"sticker"`; absent = legacy rect |
//! | `x`, `y`   | f64        | all       | post-it: top-left corner; sticker: center       |
//! | `w`, `h`   | f64        | post-it   | size in world units                             |
//! | `color`    | string     | post-it   | a [`Color`] name; absent or unknown = yellow    |
//! | `text`     | `LoroText` | post-it   | content, merged character by character          |
//! | `emoji`    | string     | sticker   | one of [`STICKERS`]                             |
//! | `scale`    | f64        | sticker   | 1 = [`STICKER_SIZE`] world units                |
//! | `rotation` | f64        | sticker   | radians, clockwise                              |
//!
//! Phase 0 boards only stored `x, y, w, h`; such entries are read as yellow
//! post-its without text, so old boards keep loading without a migration.
//! Every field is a register of its own, so concurrent edits to different
//! fields (one user moves, the other recolors) both survive.

use std::f64::consts::{PI, SQRT_2, TAU};

use loro::LoroValue;

use crate::Bounds;

/// Number of `f64` values per shape in the flat arrays handed to JS.
///
/// Layout: `[index, kind, x, y, a, b, c, d]`, where `index` is the position
/// in [`crate::Board::shapes`], `kind` is `0` (post-it) or `1` (sticker), and
/// - post-it: `a = w`, `b = h`, `c = color index`, `d = text version`;
/// - sticker: `a = scale`, `b = rotation`, `c = emoji index`, `d = 0`.
pub const RECORD: usize = 8;

/// Smallest post-it side, in world units.
pub const POSTIT_MIN_SIZE: f64 = 40.0;
/// Largest post-it side, in world units.
pub const POSTIT_MAX_SIZE: f64 = 2_000.0;
/// Longest post-it text, in Unicode scalar values. Longer input is cut.
pub const MAX_TEXT_CHARS: usize = 2_000;

/// Side of a sticker at scale 1, in world units.
pub const STICKER_SIZE: f64 = 64.0;
/// Smallest sticker scale.
pub const STICKER_MIN_SCALE: f64 = 0.25;
/// Largest sticker scale.
pub const STICKER_MAX_SCALE: f64 = 8.0;

/// Stickers offered in the tray. The document stores the emoji itself; its
/// position here is only how it travels to JS.
pub const STICKERS: [&str; 24] = [
    "👍", "❤️", "😂", "🎉", "🔥", "⭐", "✅", "❌", "❓", "💡", "👀", "🙌", "👏", "😍", "😮", "😢",
    "🤔", "💯", "🚀", "🎯", "⚡", "📌", "🏆", "☕",
];

/// Post-it colors, in toolbar order (same palette as `FigJam`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Yellow,
    Orange,
    Pink,
    Purple,
    Blue,
    Green,
}

impl Color {
    pub const ALL: [Self; 6] = [
        Self::Yellow,
        Self::Orange,
        Self::Pink,
        Self::Purple,
        Self::Blue,
        Self::Green,
    ];

    /// Name stored in the document.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Blue => "blue",
            Self::Green => "green",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    /// Position in [`Color::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Kind-specific part of a [`Shape`].
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Postit {
        w: f64,
        h: f64,
        color: Color,
        text: String,
    },
    Sticker {
        /// Index into [`STICKERS`].
        emoji: usize,
        scale: f64,
        rotation: f64,
    },
}

/// A shape as read from the document, already validated.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub body: Body,
}

impl Shape {
    /// Axis-aligned box that contains the whole shape (for a rotated sticker,
    /// the box of its circumscribed circle).
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        match self.body {
            Body::Postit { w, h, .. } => Bounds::from_rect(self.x, self.y, w, h),
            Body::Sticker { scale, .. } => {
                let r = STICKER_SIZE / 2.0 * scale * SQRT_2;
                Bounds::from_rect(self.x - r, self.y - r, 2.0 * r, 2.0 * r)
            }
        }
    }

    /// Whether the world point `(px, py)` falls on the shape. Stickers are
    /// tested in their own rotated frame.
    #[must_use]
    pub fn contains(&self, px: f64, py: f64) -> bool {
        match self.body {
            Body::Postit { .. } => self.bounds().contains(px, py),
            Body::Sticker {
                scale, rotation, ..
            } => {
                let (sin, cos) = (-rotation).sin_cos();
                let (dx, dy) = (px - self.x, py - self.y);
                let half = STICKER_SIZE / 2.0 * scale;
                (dx * cos - dy * sin).abs() <= half && (dx * sin + dy * cos).abs() <= half
            }
        }
    }

    /// Draw layer: stickers always sit on top of post-its.
    #[must_use]
    pub fn layer(&self) -> u8 {
        match self.body {
            Body::Postit { .. } => 0,
            Body::Sticker { .. } => 1,
        }
    }

    /// The shape as a flat [`RECORD`], with `index` as its first value.
    #[must_use]
    pub fn record(&self, index: usize) -> [f64; RECORD] {
        let (kind, a, b, c, d) = match &self.body {
            Body::Postit { w, h, color, text } => {
                (0.0, *w, *h, small(color.index()), text_version(text))
            }
            Body::Sticker {
                emoji,
                scale,
                rotation,
            } => (1.0, *scale, *rotation, small(*emoji), 0.0),
        };
        [small(index), kind, self.x, self.y, a, b, c, d]
    }

    /// Reads one entry of the `shapes` map. Returns `None` for entries that
    /// cannot be drawn (missing coordinates, unknown kind or emoji); numeric
    /// fields from remote peers are clamped to the valid ranges.
    pub(crate) fn parse(id: &str, value: &LoroValue) -> Option<Self> {
        let LoroValue::Map(fields) = value else {
            return None;
        };
        let num = |k: &str| match fields.get(k) {
            Some(LoroValue::Double(n)) if n.is_finite() => Some(*n),
            _ => None,
        };
        let string = |k: &str| match fields.get(k) {
            Some(LoroValue::String(s)) => Some(s.as_str()),
            _ => None,
        };
        let body = match string("kind") {
            // Legacy Phase 0 rectangles have no `kind`.
            None | Some("postit") => Body::Postit {
                w: clamp_size(num("w")?),
                h: clamp_size(num("h")?),
                color: string("color")
                    .and_then(Color::from_name)
                    .unwrap_or_default(),
                text: string("text").map(str::to_owned).unwrap_or_default(),
            },
            Some("sticker") => Body::Sticker {
                emoji: STICKERS.iter().position(|e| Some(*e) == string("emoji"))?,
                scale: clamp_scale(num("scale").unwrap_or(1.0)),
                rotation: normalize_angle(num("rotation").unwrap_or(0.0)),
            },
            Some(_) => return None,
        };
        Some(Self {
            id: id.to_owned(),
            x: num("x")?,
            y: num("y")?,
            body,
        })
    }
}

/// Clamps a post-it side to `[POSTIT_MIN_SIZE, POSTIT_MAX_SIZE]`.
#[must_use]
pub fn clamp_size(v: f64) -> f64 {
    v.clamp(POSTIT_MIN_SIZE, POSTIT_MAX_SIZE)
}

/// Clamps a sticker scale to `[STICKER_MIN_SCALE, STICKER_MAX_SCALE]`.
#[must_use]
pub fn clamp_scale(v: f64) -> f64 {
    v.clamp(STICKER_MIN_SCALE, STICKER_MAX_SCALE)
}

/// Maps any angle to `(-π, π]`.
#[must_use]
pub fn normalize_angle(a: f64) -> f64 {
    let r = a.rem_euclid(TAU);
    if r > PI { r - TAU } else { r }
}

/// Small non-negative integers (indices, palette positions) as `f64`.
fn small(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Content hash of a post-it text, so JS can cache its layout and only ask
/// for the text when it changes. `0` means empty; otherwise a 53-bit FNV-1a
/// hash, which an `f64` represents exactly.
#[must_use]
pub fn text_version(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    let h = (h >> 11).max(1);
    // Exact: `h` has at most 53 significant bits.
    #[allow(clippy::cast_precision_loss)]
    let v = h as f64;
    v
}

/// Smallest single edit turning `old` into `new`: `(position, deleted, inserted)`
/// in Unicode scalar values, or `None` if they are equal. Typing, pasting and
/// deleting a selection are one contiguous change, which this captures exactly;
/// expressing it as a splice (instead of replacing the whole text) is what lets
/// concurrent edits from two users merge character by character.
#[must_use]
pub fn text_diff<'a>(old: &str, new: &'a str) -> Option<(usize, usize, &'a str)> {
    if old == new {
        return None;
    }
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let old_len = old.chars().count();
    let new_len = new.chars().count();
    let max_suffix = old_len.min(new_len) - prefix;
    let suffix = old
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let start = new.char_indices().nth(prefix).map_or(new.len(), |(i, _)| i);
    let end = new
        .char_indices()
        .nth(new_len - suffix)
        .map_or(new.len(), |(i, _)| i);
    Some((prefix, old_len - prefix - suffix, &new[start..end]))
}

/// Cuts `text` to at most [`MAX_TEXT_CHARS`] Unicode scalar values.
#[must_use]
pub fn truncate_text(text: &str) -> &str {
    text.char_indices()
        .nth(MAX_TEXT_CHARS)
        .map_or(text, |(i, _)| &text[..i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sticker(x: f64, y: f64, scale: f64, rotation: f64) -> Shape {
        Shape {
            id: "s".into(),
            x,
            y,
            body: Body::Sticker {
                emoji: 0,
                scale,
                rotation,
            },
        }
    }

    #[test]
    fn color_names_round_trip() {
        for (i, c) in Color::ALL.into_iter().enumerate() {
            assert_eq!(Color::from_name(c.name()), Some(c));
            assert_eq!(c.index(), i);
        }
        assert_eq!(Color::from_name("magenta"), None);
        assert_eq!(Color::default(), Color::Yellow);
    }

    #[test]
    fn sticker_palette_has_no_duplicates() {
        let mut seen = STICKERS.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), STICKERS.len());
    }

    #[test]
    fn sticker_hit_test_follows_rotation() {
        // 64x64 at scale 1, centered on the origin.
        let s = sticker(0.0, 0.0, 1.0, 0.0);
        assert!(s.contains(30.0, 30.0));
        assert!(!s.contains(40.0, 0.0));
        // Rotated 45°, the corner (30, 30) now falls outside, while a point
        // 40 units along the x axis falls inside (half-diagonal ≈ 45).
        let r = sticker(0.0, 0.0, 1.0, PI / 4.0);
        assert!(!r.contains(30.0, 30.0));
        assert!(r.contains(40.0, 0.0));
        // Bounds cover every rotation.
        let b = r.bounds();
        assert!(b.contains(45.0, 0.0) && b.contains(-45.0, -45.0));
    }

    #[test]
    fn sticker_scale_grows_the_hit_area() {
        let s = sticker(100.0, 100.0, 2.0, 0.0);
        assert!(s.contains(160.0, 160.0));
        assert!(!s.contains(170.0, 100.0));
    }

    #[test]
    fn angles_are_normalized() {
        assert!((normalize_angle(3.0 * PI) - PI).abs() < 1e-9);
        assert!((normalize_angle(-PI / 2.0) + PI / 2.0).abs() < 1e-9);
        assert!((normalize_angle(TAU + 0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clamps_sizes_and_scales() {
        assert_eq!(clamp_size(1.0), POSTIT_MIN_SIZE);
        assert_eq!(clamp_size(1e9), POSTIT_MAX_SIZE);
        assert_eq!(clamp_size(150.0), 150.0);
        assert_eq!(clamp_scale(0.0), STICKER_MIN_SCALE);
        assert_eq!(clamp_scale(100.0), STICKER_MAX_SCALE);
    }

    #[test]
    fn text_version_tracks_content() {
        assert_eq!(text_version(""), 0.0);
        let a = text_version("abc");
        assert!(a > 0.0 && a < 2f64.powi(53));
        assert_eq!(a, text_version("abc"));
        assert_ne!(a, text_version("abd"));
    }

    #[test]
    fn text_diff_finds_the_single_edit() {
        assert_eq!(text_diff("abc", "abc"), None);
        assert_eq!(text_diff("", "hi"), Some((0, 0, "hi")));
        assert_eq!(text_diff("hello", "help"), Some((3, 2, "p")));
        assert_eq!(text_diff("hello", "hello world"), Some((5, 0, " world")));
        assert_eq!(text_diff("aaa", "aa"), Some((2, 1, "")));
        assert_eq!(text_diff("ação", "açúcar"), Some((2, 2, "úcar")));
        assert_eq!(text_diff("a😀b", "a😀😀b"), Some((2, 0, "😀")));
    }

    #[test]
    fn long_text_is_truncated() {
        let long = "é".repeat(MAX_TEXT_CHARS + 10);
        assert_eq!(truncate_text(&long).chars().count(), MAX_TEXT_CHARS);
        assert_eq!(truncate_text("curto"), "curto");
    }

    #[test]
    fn records_are_flat_and_typed() {
        let p = Shape {
            id: "p".into(),
            x: 1.0,
            y: 2.0,
            body: Body::Postit {
                w: 3.0,
                h: 4.0,
                color: Color::Blue,
                text: String::new(),
            },
        };
        assert_eq!(p.record(7), [7.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 0.0]);
        let s = sticker(5.0, 6.0, 2.0, 0.5);
        assert_eq!(s.record(0), [0.0, 1.0, 5.0, 6.0, 2.0, 0.5, 0.0, 0.0]);
        assert_eq!(p.layer(), 0);
        assert_eq!(s.layer(), 1);
    }
}

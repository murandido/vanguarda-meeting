//! JS ↔ WASM bridge. Thin on purpose: JS only forwards events and bytes;
//! state and logic live in Rust.

use wasm_bindgen::prelude::*;

use crate::{Board, BoardError, Camera, Color, STICKERS, shape};

mod connectors;

/// Margem, em pixels de tela, ao enquadrar o conteúdo (`Shift+1`).
const FIT_MARGIN: f64 = 48.0;

#[wasm_bindgen(js_name = Board)]
pub struct JsBoard(Board);

#[wasm_bindgen(js_class = Board)]
impl JsBoard {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self(Board::new())
    }

    #[wasm_bindgen(js_name = fromSnapshot)]
    pub fn from_snapshot(bytes: &[u8]) -> Result<JsBoard, JsError> {
        Ok(Self(Board::from_snapshot(bytes)?))
    }

    /// Creates an empty post-it with its top-left corner at `(x, y)`.
    /// `color` is a palette name (see `colorNames()`).
    #[wasm_bindgen(js_name = addPostit)]
    pub fn add_postit(
        &self,
        id: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        color: &str,
    ) -> Result<(), JsError> {
        Ok(self.0.add_postit(id, x, y, w, h, parse_color(color)?)?)
    }

    /// Stamps a sticker (one of `stickerPalette()`) centered on `(x, y)`.
    #[wasm_bindgen(js_name = addSticker)]
    pub fn add_sticker(&self, id: &str, emoji: &str, x: f64, y: f64) -> Result<(), JsError> {
        Ok(self.0.add_sticker(id, emoji, x, y)?)
    }

    #[wasm_bindgen(js_name = moveShape)]
    pub fn move_shape(&self, id: &str, x: f64, y: f64) -> Result<(), JsError> {
        Ok(self.0.move_shape(id, x, y)?)
    }

    #[wasm_bindgen(js_name = resizePostit)]
    pub fn resize_postit(&self, id: &str, w: f64, h: f64) -> Result<(), JsError> {
        Ok(self.0.resize_postit(id, w, h)?)
    }

    #[wasm_bindgen(js_name = setColor)]
    pub fn set_color(&self, id: &str, color: &str) -> Result<(), JsError> {
        Ok(self.0.set_color(id, parse_color(color)?)?)
    }

    /// Replaces a post-it's text; only the changed characters are recorded.
    #[wasm_bindgen(js_name = setText)]
    pub fn set_text(&self, id: &str, text: &str) -> Result<(), JsError> {
        Ok(self.0.set_text(id, text)?)
    }

    pub fn text(&self, id: &str) -> Result<String, JsError> {
        Ok(self.0.text(id)?)
    }

    /// Text of the shape at `index` (the first value of a record).
    #[wasm_bindgen(js_name = textAt)]
    #[must_use]
    pub fn text_at(&self, index: usize) -> String {
        self.0.text_at(index)
    }

    #[wasm_bindgen(js_name = transformSticker)]
    pub fn transform_sticker(&self, id: &str, scale: f64, rotation: f64) -> Result<(), JsError> {
        Ok(self.0.transform_sticker(id, scale, rotation)?)
    }

    #[wasm_bindgen(js_name = removeShape)]
    pub fn remove_shape(&self, id: &str) {
        self.0.remove_shape(id);
    }

    pub fn import(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        Ok(self.0.import(bytes)?)
    }

    /// Bytes to send over the WebSocket, or `undefined` if nothing changed.
    #[wasm_bindgen(js_name = takePendingUpdate)]
    pub fn take_pending_update(&mut self) -> Result<Option<Vec<u8>>, JsError> {
        Ok(self.0.take_pending_update()?)
    }

    /// Shapes visible to the camera on a `width` x `height` CSS-pixel screen,
    /// as one flat array of records (`RECORD` values each, world coordinates,
    /// draw order). One JS ↔ WASM crossing per frame; see [`crate::RECORD`]
    /// for the layout.
    #[wasm_bindgen(js_name = visibleShapes)]
    #[must_use]
    pub fn visible_shapes(&self, camera: &JsCamera, width: f64, height: f64) -> Vec<f64> {
        self.0.visible_records(&camera.0.viewport(width, height))
    }

    /// Record of one shape (same layout), or `undefined` if it is gone.
    #[wasm_bindgen(js_name = shapeRecord)]
    #[must_use]
    pub fn shape_record(&self, id: &str) -> Option<Vec<f64>> {
        self.0.with_shapes(|shapes| {
            shapes
                .iter()
                .enumerate()
                .find(|(_, s)| s.id == id)
                .map(|(i, s)| s.record(i).to_vec())
        })
    }

    /// Id of the topmost shape under the world point `(x, y)`, if any.
    #[wasm_bindgen(js_name = hitTest)]
    #[must_use]
    pub fn hit_test(&self, x: f64, y: f64) -> Option<String> {
        self.0.hit_test(x, y).map(|s| s.id)
    }
}

impl Default for JsBoard {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_color(name: &str) -> Result<Color, JsError> {
    Color::from_name(name).ok_or_else(|| BoardError::UnknownColor(name.to_owned()).into())
}

/// Post-it color names, in palette order (the `c` value of a post-it record).
#[wasm_bindgen(js_name = colorNames)]
#[must_use]
pub fn color_names() -> Vec<String> {
    Color::ALL.iter().map(|c| c.name().to_owned()).collect()
}

/// Sticker emojis, in tray order (the `c` value of a sticker record).
#[wasm_bindgen(js_name = stickerPalette)]
#[must_use]
pub fn sticker_palette() -> Vec<String> {
    STICKERS.iter().map(|&e| e.to_owned()).collect()
}

/// Size limits the UI applies while dragging handles, so the preview matches
/// what the document will store: `[postitMin, postitMax, stickerSize,
/// stickerMinScale, stickerMaxScale, maxTextChars]`.
#[wasm_bindgen(js_name = shapeLimits)]
#[must_use]
pub fn shape_limits() -> Vec<f64> {
    vec![
        shape::POSTIT_MIN_SIZE,
        shape::POSTIT_MAX_SIZE,
        shape::STICKER_SIZE,
        shape::STICKER_MIN_SCALE,
        shape::STICKER_MAX_SCALE,
        f64::from(u32::try_from(shape::MAX_TEXT_CHARS).unwrap_or(u32::MAX)),
    ]
}

/// Câmera local (pan + zoom). Não vai para o documento: cada participante tem
/// a sua. Pontos de tela são pixels CSS relativos ao canvas.
#[wasm_bindgen(js_name = Camera)]
pub struct JsCamera(Camera);

#[wasm_bindgen(js_class = Camera)]
impl JsCamera {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        Self(Camera::default())
    }

    /// Ponto do mundo no canto superior esquerdo da tela.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn x(&self) -> f64 {
        self.0.x()
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn y(&self) -> f64 {
        self.0.y()
    }

    /// Pixels de tela por unidade do mundo (1 = 100%).
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn zoom(&self) -> f64 {
        self.0.zoom()
    }

    /// A transformação é separável por eixo, então cada coordenada converte
    /// sozinha — sem alocar arrays a cada `pointermove`.
    #[wasm_bindgen(js_name = toWorldX)]
    #[must_use]
    pub fn to_world_x(&self, sx: f64) -> f64 {
        self.0.screen_to_world(sx, 0.0).0
    }

    #[wasm_bindgen(js_name = toWorldY)]
    #[must_use]
    pub fn to_world_y(&self, sy: f64) -> f64 {
        self.0.screen_to_world(0.0, sy).1
    }

    #[wasm_bindgen(js_name = toScreenX)]
    #[must_use]
    pub fn to_screen_x(&self, wx: f64) -> f64 {
        self.0.world_to_screen(wx, 0.0).0
    }

    #[wasm_bindgen(js_name = toScreenY)]
    #[must_use]
    pub fn to_screen_y(&self, wy: f64) -> f64 {
        self.0.world_to_screen(0.0, wy).1
    }

    /// Arrasta o conteúdo `(dx, dy)` pixels de tela.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.0.pan(dx, dy);
    }

    /// Zoom de um `WheelEvent` (Ctrl/Cmd + roda ou pinça) em torno de `(sx, sy)`.
    #[wasm_bindgen(js_name = wheelZoom)]
    pub fn wheel_zoom(&mut self, sx: f64, sy: f64, delta_y: f64, delta_mode: u32) {
        self.0.wheel_zoom(sx, sy, delta_y, delta_mode);
    }

    /// Volta para 100% mantendo o centro da tela.
    #[wasm_bindgen(js_name = resetZoom)]
    pub fn reset_zoom(&mut self, width: f64, height: f64) {
        self.0.reset_zoom(width, height);
    }

    /// Enquadra todo o conteúdo do quadro; num quadro vazio, volta para 100%.
    #[wasm_bindgen(js_name = zoomToFit)]
    pub fn zoom_to_fit(&mut self, board: &JsBoard, width: f64, height: f64) {
        match board.0.content_bounds() {
            Some(content) => self.0.fit(&content, width, height, FIT_MARGIN),
            None => self.0.reset_zoom(width, height),
        }
    }
}

impl Default for JsCamera {
    fn default() -> Self {
        Self::new()
    }
}

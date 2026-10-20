//! Ponte JS ↔ WASM. Fica fina de propósito: o JS só repassa eventos e bytes;
//! estado e lógica moram no Rust.

use wasm_bindgen::prelude::*;

use crate::{Board, Camera};

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

    #[wasm_bindgen(js_name = addRect)]
    pub fn add_rect(&self, id: &str, x: f64, y: f64, w: f64, h: f64) {
        self.0.add_rect(id, x, y, w, h);
    }

    #[wasm_bindgen(js_name = moveShape)]
    pub fn move_shape(&self, id: &str, x: f64, y: f64) -> Result<(), JsError> {
        Ok(self.0.move_shape(id, x, y)?)
    }

    /// Reverts this user's last step (never the partner's). The change goes
    /// out through `takePendingUpdate()` like any edit.
    pub fn undo(&mut self) -> bool {
        self.0.undo()
    }

    pub fn redo(&mut self) -> bool {
        self.0.redo()
    }

    /// Edits until `endGroup()` undo as one step (a drag, a resize, a text
    /// editing session). See `history.rs` for the policy.
    #[wasm_bindgen(js_name = beginGroup)]
    pub fn begin_group(&mut self) {
        self.0.begin_group();
    }

    #[wasm_bindgen(js_name = endGroup)]
    pub fn end_group(&mut self) {
        self.0.end_group();
    }

    pub fn import(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        Ok(self.0.import(bytes)?)
    }

    /// Bytes a enviar pelo WebSocket, ou `undefined` se nada mudou.
    #[wasm_bindgen(js_name = takePendingUpdate)]
    pub fn take_pending_update(&mut self) -> Result<Option<Vec<u8>>, JsError> {
        Ok(self.0.take_pending_update()?)
    }

    /// Retângulos como array plano `[x, y, w, h, x, y, w, h, ...]`, na mesma
    /// ordem de `rectIds()` — evita criar um objeto JS por forma a cada frame.
    #[wasm_bindgen(js_name = rectGeometry)]
    #[must_use]
    pub fn rect_geometry(&self) -> Vec<f64> {
        self.0
            .with_rects(|rects| rects.iter().flat_map(|r| [r.x, r.y, r.w, r.h]).collect())
    }

    #[wasm_bindgen(js_name = rectIds)]
    #[must_use]
    pub fn rect_ids(&self) -> Vec<String> {
        self.0
            .with_rects(|rects| rects.iter().map(|r| r.id.clone()).collect())
    }

    /// Só os retângulos visíveis pela câmera numa tela de `width` x `height`
    /// pixels CSS, no mesmo formato plano de `rectGeometry()` (coordenadas do
    /// mundo, ordem de desenho). Uma travessia JS ↔ WASM por frame.
    #[wasm_bindgen(js_name = visibleGeometry)]
    #[must_use]
    pub fn visible_geometry(&self, camera: &JsCamera, width: f64, height: f64) -> Vec<f64> {
        self.0.visible_geometry(&camera.0.viewport(width, height))
    }

    /// Retângulo mais acima sob o ponto do mundo `(x, y)`, se houver.
    #[wasm_bindgen(js_name = hitTest)]
    #[must_use]
    pub fn hit_test(&self, x: f64, y: f64) -> Option<Hit> {
        self.0.hit_test(x, y).map(|r| Hit {
            id: r.id,
            x: r.x,
            y: r.y,
        })
    }
}

impl Default for JsBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// Resultado de `Board.hitTest`: id e origem (mundo) da forma atingida.
#[wasm_bindgen]
pub struct Hit {
    #[wasm_bindgen(getter_with_clone)]
    pub id: String,
    pub x: f64,
    pub y: f64,
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

//! Ponte JS ↔ WASM. Fica fina de propósito: o JS só repassa eventos e bytes;
//! estado e lógica moram no Rust.

use wasm_bindgen::prelude::*;

use crate::Board;

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
            .rects()
            .iter()
            .flat_map(|r| [r.x, r.y, r.w, r.h])
            .collect()
    }

    #[wasm_bindgen(js_name = rectIds)]
    #[must_use]
    pub fn rect_ids(&self) -> Vec<String> {
        self.0.rects().into_iter().map(|r| r.id).collect()
    }
}

impl Default for JsBoard {
    fn default() -> Self {
        Self::new()
    }
}

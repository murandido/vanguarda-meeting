//! Motor do Cartazes: estado do quadro num documento Loro (CRDT).
//!
//! Na Fase 0 o quadro só tem retângulos móveis — o suficiente para provar
//! a sincronização entre dois navegadores. A ponte para o navegador fica em
//! [`wasm`], compilada apenas para `wasm32`.

use loro::{ExportMode, LoroDoc, LoroMap, LoroValue, VersionVector};

#[cfg(target_arch = "wasm32")]
pub mod wasm;

const SHAPES: &str = "shapes";

#[derive(Debug, thiserror::Error)]
pub enum BoardError {
    #[error("atualização ou snapshot inválido: {0}")]
    Import(String),
    #[error("falha ao exportar o documento: {0}")]
    Export(String),
    #[error("forma inexistente: {0}")]
    UnknownShape(String),
}

/// Retângulo como lido do documento.
#[derive(Debug, Clone, PartialEq)]
pub struct Rect {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Um quadro: o documento Loro mais o ponto até onde as mudanças locais já
/// foram enviadas.
pub struct Board {
    doc: LoroDoc,
    sent: VersionVector,
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
        Self { doc, sent }
    }

    /// Recria um quadro a partir de um snapshot salvo no servidor.
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, BoardError> {
        let mut board = Self::new();
        board.import(bytes)?;
        board.sent = board.doc.oplog_vv();
        Ok(board)
    }

    /// Fixa o peer id; só necessário em testes que precisam de determinismo.
    pub fn set_peer_id(&self, peer: u64) {
        self.doc
            .set_peer_id(peer)
            .expect("peer id definido antes de qualquer edição");
    }

    fn shapes(&self) -> LoroMap {
        self.doc.get_map(SHAPES)
    }

    pub fn add_rect(&self, id: &str, x: f64, y: f64, w: f64, h: f64) {
        let shape = self
            .shapes()
            .insert_container(id, LoroMap::new())
            .expect("chave de mapa válida");
        for (key, value) in [("x", x), ("y", y), ("w", w), ("h", h)] {
            shape.insert(key, value).expect("valor escalar");
        }
        self.doc.commit();
    }

    pub fn move_shape(&self, id: &str, x: f64, y: f64) -> Result<(), BoardError> {
        let shape = self
            .shapes()
            .get(id)
            .and_then(|v| v.into_container().ok())
            .and_then(|c| c.into_map().ok())
            .ok_or_else(|| BoardError::UnknownShape(id.to_owned()))?;
        shape.insert("x", x).expect("valor escalar");
        shape.insert("y", y).expect("valor escalar");
        self.doc.commit();
        Ok(())
    }

    pub fn remove_shape(&self, id: &str) {
        self.shapes().delete(id).expect("chave de mapa válida");
        self.doc.commit();
    }

    /// Todos os retângulos, ordenados por id para leitura determinística.
    #[must_use]
    pub fn rects(&self) -> Vec<Rect> {
        let LoroValue::Map(shapes) = self.shapes().get_deep_value() else {
            return Vec::new();
        };
        let mut rects: Vec<Rect> = shapes
            .iter()
            .filter_map(|(id, value)| {
                let LoroValue::Map(fields) = value else {
                    return None;
                };
                let num = |k: &str| match fields.get(k) {
                    Some(LoroValue::Double(n)) => Some(*n),
                    _ => None,
                };
                Some(Rect {
                    id: id.clone(),
                    x: num("x")?,
                    y: num("y")?,
                    w: num("w")?,
                    h: num("h")?,
                })
            })
            .collect();
        rects.sort_by(|a, b| a.id.cmp(&b.id));
        rects
    }

    /// Aplica uma atualização ou snapshot vindo de outro participante.
    pub fn import(&mut self, bytes: &[u8]) -> Result<(), BoardError> {
        self.doc
            .import(bytes)
            .map(|_| ())
            .map_err(|e| BoardError::Import(e.to_string()))
    }

    /// Mudanças ainda não enviadas, ou `None` se não houver nenhuma.
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
mod tests {
    use super::*;

    #[test]
    fn add_and_move_rect() {
        let board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 20.0);
        board.move_shape("a", 5.0, 6.0).unwrap();
        assert_eq!(
            board.rects(),
            vec![Rect {
                id: "a".into(),
                x: 5.0,
                y: 6.0,
                w: 10.0,
                h: 20.0
            }]
        );
    }

    #[test]
    fn moving_unknown_shape_fails() {
        let board = Board::new();
        assert!(matches!(
            board.move_shape("nada", 1.0, 1.0),
            Err(BoardError::UnknownShape(_))
        ));
    }

    #[test]
    fn pending_update_is_taken_once() {
        let mut board = Board::new();
        assert!(board.take_pending_update().unwrap().is_none());
        board.add_rect("a", 0.0, 0.0, 1.0, 1.0);
        assert!(board.take_pending_update().unwrap().is_some());
        assert!(board.take_pending_update().unwrap().is_none());
    }

    #[test]
    fn snapshot_restores_state() {
        let board = Board::new();
        board.add_rect("a", 1.0, 2.0, 3.0, 4.0);
        let restored = Board::from_snapshot(&board.snapshot().unwrap()).unwrap();
        assert_eq!(restored.rects(), board.rects());
    }
}

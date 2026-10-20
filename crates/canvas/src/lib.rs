//! Motor do Cartazes: estado do quadro num documento Loro (CRDT).
//!
//! Na Fase 0 o quadro só tem retângulos móveis — o suficiente para provar
//! a sincronização entre dois navegadores. A câmera (pan/zoom, local de cada
//! participante) fica em [`camera`]; a ponte para o navegador fica em
//! [`wasm`], compilada apenas para `wasm32`.

use std::cell::RefCell;

use loro::{ExportMode, Frontiers, LoroDoc, LoroMap, LoroValue, VersionVector};

pub mod camera;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use camera::{Bounds, Camera};

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

impl Rect {
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        Bounds::from_rect(self.x, self.y, self.w, self.h)
    }
}

/// Retângulos que tocam `view`, na ordem de desenho de `rects`.
pub fn cull<'a>(rects: &'a [Rect], view: &'a Bounds) -> impl Iterator<Item = &'a Rect> + 'a {
    rects.iter().filter(move |r| r.bounds().intersects(view))
}

/// Um quadro: o documento Loro mais o ponto até onde as mudanças locais já
/// foram enviadas.
pub struct Board {
    doc: LoroDoc,
    sent: VersionVector,
    /// Retângulos já lidos do documento, válidos enquanto as frontiers do
    /// estado não mudarem. Reler o documento inteiro a cada frame custaria
    /// caro com milhares de formas; comparar frontiers é barato.
    cache: RefCell<Option<(Frontiers, Vec<Rect>)>>,
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
        }
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
    /// Essa também é a ordem de desenho: o último fica por cima.
    #[must_use]
    pub fn rects(&self) -> Vec<Rect> {
        self.with_rects(<[Rect]>::to_vec)
    }

    /// Executa `f` sobre os retângulos atuais, relendo o documento só se ele
    /// mudou desde a última leitura.
    pub fn with_rects<R>(&self, f: impl FnOnce(&[Rect]) -> R) -> R {
        let now = self.doc.state_frontiers();
        let mut cache = self.cache.borrow_mut();
        if cache.as_ref().is_none_or(|(at, _)| *at != now) {
            *cache = Some((now, self.read_rects()));
        }
        let (_, rects) = cache.as_ref().expect("cache preenchido acima");
        f(rects)
    }

    /// Geometria dos retângulos visíveis em `view`, como array plano
    /// `[x, y, w, h, ...]` na ordem de desenho.
    #[must_use]
    pub fn visible_geometry(&self, view: &Bounds) -> Vec<f64> {
        self.with_rects(|rects| {
            cull(rects, view)
                .flat_map(|r| [r.x, r.y, r.w, r.h])
                .collect()
        })
    }

    /// Retângulo mais acima (último na ordem de desenho) sob o ponto do mundo.
    #[must_use]
    pub fn hit_test(&self, x: f64, y: f64) -> Option<Rect> {
        self.with_rects(|rects| {
            rects
                .iter()
                .rev()
                .find(|r| r.bounds().contains(x, y))
                .cloned()
        })
    }

    /// Caixa que envolve todo o conteúdo, ou `None` num quadro vazio.
    #[must_use]
    pub fn content_bounds(&self) -> Option<Bounds> {
        self.with_rects(|rects| rects.iter().map(Rect::bounds).reduce(|a, b| a.union(&b)))
    }

    fn read_rects(&self) -> Vec<Rect> {
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

    #[test]
    fn cache_follows_local_and_remote_changes() {
        let mut a = Board::new();
        let b = Board::new();
        a.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        assert_eq!(a.rects().len(), 1);
        a.move_shape("a", 5.0, 5.0).unwrap();
        assert_eq!(a.rects()[0].x, 5.0);
        b.add_rect("b", 1.0, 1.0, 1.0, 1.0);
        let mut b = b;
        a.import(&b.take_pending_update().unwrap().unwrap())
            .unwrap();
        assert_eq!(a.rects().len(), 2);
        a.remove_shape("a");
        assert_eq!(
            a.rects().iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
    }

    #[test]
    fn visible_geometry_culls_to_viewport() {
        let board = Board::new();
        board.add_rect("a", 0.0, 0.0, 10.0, 10.0);
        board.add_rect("b", 500.0, 500.0, 10.0, 10.0);
        board.add_rect("c", -50.0, -50.0, 45.0, 45.0);
        let view = Camera::default().viewport(100.0, 100.0);
        assert_eq!(board.visible_geometry(&view), vec![0.0, 0.0, 10.0, 10.0]);
        let far = Camera::new(400.0, 400.0, 1.0).viewport(200.0, 200.0);
        assert_eq!(board.visible_geometry(&far), vec![500.0, 500.0, 10.0, 10.0]);
        let all = Camera::new(-100.0, -100.0, 0.1).viewport(1000.0, 1000.0);
        assert_eq!(board.visible_geometry(&all).len(), 12);
    }

    #[test]
    fn cull_keeps_draw_order() {
        let rects: Vec<Rect> = (0..5)
            .map(|i| Rect {
                id: format!("r{i}"),
                x: f64::from(i) * 100.0,
                y: 0.0,
                w: 50.0,
                h: 50.0,
            })
            .collect();
        let view = Bounds::from_rect(160.0, 10.0, 200.0, 10.0);
        let ids: Vec<_> = cull(&rects, &view).map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["r2", "r3"]);
    }

    #[test]
    fn hit_test_returns_topmost() {
        let board = Board::new();
        assert!(board.hit_test(0.0, 0.0).is_none());
        board.add_rect("a", 0.0, 0.0, 100.0, 100.0);
        board.add_rect("b", 50.0, 50.0, 100.0, 100.0);
        assert_eq!(board.hit_test(10.0, 10.0).unwrap().id, "a");
        assert_eq!(board.hit_test(60.0, 60.0).unwrap().id, "b");
        assert!(board.hit_test(200.0, 10.0).is_none());
    }

    #[test]
    fn content_bounds_wraps_everything() {
        let board = Board::new();
        assert!(board.content_bounds().is_none());
        board.add_rect("a", -10.0, 5.0, 20.0, 5.0);
        board.add_rect("b", 100.0, -40.0, 10.0, 10.0);
        assert_eq!(
            board.content_bounds(),
            Some(Bounds {
                min_x: -10.0,
                min_y: -40.0,
                max_x: 110.0,
                max_y: 10.0
            })
        );
    }
}

//! Limites de tamanho do quadro, aplicados pelo servidor antes de aceitar uma
//! atualização vinda de um cliente.
//!
//! A atualização é importada primeiro numa cópia (`fork`) do documento; só se
//! o resultado respeitar os [`Limits`] ela entra no documento de verdade.

use loro::{ExportMode, LoroDoc};

use crate::{Board, BoardError, SHAPES};

/// Limites do quadro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Máximo de formas no quadro.
    pub max_shapes: usize,
    /// Tamanho máximo do id de uma forma, em bytes.
    pub max_shape_id: usize,
    /// Tamanho máximo do snapshot resultante, em bytes.
    pub max_snapshot_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_shapes: 5_000,
            max_shape_id: 64,
            max_snapshot_bytes: 2 << 20,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    #[error(transparent)]
    Board(#[from] BoardError),
    #[error("formas demais: {0} (máximo {1})")]
    TooManyShapes(usize, usize),
    #[error("id de forma longo demais: {0} bytes (máximo {1})")]
    ShapeIdTooLong(usize, usize),
    #[error("snapshot grande demais: {0} bytes (máximo {1})")]
    SnapshotTooLarge(usize, usize),
}

impl Limits {
    /// Confere um documento contra os limites.
    fn check(&self, doc: &LoroDoc) -> Result<(), LimitError> {
        let shapes = doc.get_map(SHAPES);
        let count = shapes.len();
        if count > self.max_shapes {
            return Err(LimitError::TooManyShapes(count, self.max_shapes));
        }
        if let Some(len) = shapes
            .keys()
            .map(|k| k.len())
            .find(|len| *len > self.max_shape_id)
        {
            return Err(LimitError::ShapeIdTooLong(len, self.max_shape_id));
        }
        let size = doc
            .export(ExportMode::Snapshot)
            .map_err(|e| BoardError::Export(e.to_string()))?
            .len();
        if size > self.max_snapshot_bytes {
            return Err(LimitError::SnapshotTooLarge(size, self.max_snapshot_bytes));
        }
        Ok(())
    }
}

impl Board {
    /// Como [`Board::import`], mas só aplica a atualização se o quadro
    /// resultante respeitar `limits`; caso contrário o quadro fica intacto.
    pub fn import_within(&mut self, bytes: &[u8], limits: &Limits) -> Result<(), LimitError> {
        let trial = self.doc.fork();
        trial
            .import(bytes)
            .map_err(|e| BoardError::Import(e.to_string()))?;
        limits.check(&trial)?;
        self.import(bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(f: impl FnOnce(&Board)) -> Vec<u8> {
        let mut board = Board::new();
        f(&board);
        board.take_pending_update().unwrap().unwrap()
    }

    #[test]
    fn accepts_updates_within_limits() {
        let mut server = Board::new();
        let bytes = update(|b| b.add_rect("r1", 1.0, 2.0, 3.0, 4.0));
        server.import_within(&bytes, &Limits::default()).unwrap();
        assert_eq!(server.shapes().len(), 1);
    }

    #[test]
    fn rejects_garbage_without_touching_the_board() {
        let mut server = Board::new();
        let err = server
            .import_within(&[0xff; 64], &Limits::default())
            .unwrap_err();
        assert!(matches!(err, LimitError::Board(BoardError::Import(_))));
        assert_eq!(server.shapes(), Vec::new());
    }

    #[test]
    fn rejects_too_many_shapes() {
        let limits = Limits {
            max_shapes: 2,
            ..Limits::default()
        };
        let mut server = Board::new();
        let bytes = update(|b| {
            for i in 0..3 {
                b.add_rect(&format!("r{i}"), 0.0, 0.0, 1.0, 1.0);
            }
        });
        let err = server.import_within(&bytes, &limits).unwrap_err();
        assert!(matches!(err, LimitError::TooManyShapes(3, 2)));
        assert_eq!(server.shapes(), Vec::new());
    }

    #[test]
    fn rejects_long_shape_ids() {
        let mut server = Board::new();
        let bytes = update(|b| b.add_rect(&"x".repeat(65), 0.0, 0.0, 1.0, 1.0));
        let err = server
            .import_within(&bytes, &Limits::default())
            .unwrap_err();
        assert!(matches!(err, LimitError::ShapeIdTooLong(65, 64)));
        assert_eq!(server.shapes(), Vec::new());
    }

    #[test]
    fn rejects_oversized_snapshot() {
        let limits = Limits {
            max_snapshot_bytes: 200,
            ..Limits::default()
        };
        let mut server = Board::new();
        let bytes = update(|b| {
            for i in 0..50 {
                b.add_rect(&format!("r{i}"), 0.0, 0.0, 1.0, 1.0);
            }
        });
        let err = server.import_within(&bytes, &limits).unwrap_err();
        assert!(matches!(err, LimitError::SnapshotTooLarge(_, 200)));
        assert_eq!(server.shapes(), Vec::new());
        assert!(err.to_string().contains("snapshot grande demais"));
    }
}

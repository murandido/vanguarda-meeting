//! Tipos compartilhados entre o servidor e o cliente WASM.
//!
//! Protocolo do WebSocket de um quadro (`/ws/{board}`):
//! - frames **binários** carregam atualizações do Loro, repassadas como estão;
//! - frames de **texto** carregam [`PresenceMsg`] em JSON, efêmeros e nunca salvos.

use serde::{Deserialize, Serialize};

/// Mensagem efêmera de presença: cursor e saída de um participante.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PresenceMsg {
    /// Posição do cursor em coordenadas do quadro (não da tela).
    Cursor { user: String, x: f64, y: f64 },
    /// O participante saiu do quadro.
    Leave { user: String },
}

/// Valida o nome de um quadro vindo da URL: 1–64 caracteres `[a-z0-9-]`.
#[must_use]
pub fn is_valid_board_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_roundtrips_as_tagged_json() {
        let msg = PresenceMsg::Cursor {
            user: "ana".into(),
            x: 1.5,
            y: -2.0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"cursor","user":"ana","x":1.5,"y":-2.0}"#);
        assert_eq!(serde_json::from_str::<PresenceMsg>(&json).unwrap(), msg);
    }

    #[test]
    fn board_ids() {
        assert!(is_valid_board_id("reuniao-2026-10-20"));
        assert!(!is_valid_board_id(""));
        assert!(!is_valid_board_id("Maiuscula"));
        assert!(!is_valid_board_id("../etc"));
        assert!(!is_valid_board_id(&"a".repeat(65)));
    }
}

pub fn   probe_fmt() {}

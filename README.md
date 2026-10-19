# Vanguarda

Dashboard da dupla para a reunião semanal de alinhamento:

- **Cartazes** — quadro infinito colaborativo estilo FigJam (Rust → WASM + Loro).
- **Ritmo** — Kanban semanal com check-in diário (HTMX renderizado no servidor).

Requisitos, decisões e roadmap: [documento de requisitos](https://claude.ai/code/artifact/3e365f49-9a74-4fd3-9694-bfc144baf029).

## Estrutura

| Caminho | O que é |
| --- | --- |
| `crates/shared` | Tipos do protocolo (presença, validação de nomes) |
| `crates/canvas` | Motor do Cartazes: estado em Loro; ponte JS em `src/wasm.rs` |
| `crates/server` | Axum: páginas HTMX/Askama, relay WebSocket `/ws/{quadro}`, SQLite |
| `e2e/` | Playwright: dois navegadores no mesmo quadro |
| `.sqlx/` | Metadados offline das queries (`cargo sqlx prepare --workspace`) |

## Rodando localmente

Pré-requisitos: Rust estável com `rustfmt`, `clippy` e o target `wasm32-unknown-unknown`;
`wasm-pack`; `sqlx-cli`; Node 22 (para o E2E); `lefthook`.

```sh
lefthook install                                   # hooks de pre-commit
wasm-pack build crates/canvas --target web --release
cargo run -p server                                # http://127.0.0.1:3000
```

Variáveis do servidor: `DATABASE_URL` (padrão `sqlite://vanguarda.db`), `BIND`
(padrão `127.0.0.1:3000`), `STATIC_DIR`, `PKG_DIR`.

Abra `/b/teste` em duas abas: duplo clique cria um retângulo, arrastar move,
e a outra aba acompanha.

## Quality gates

Tudo roda no CI (`.github/workflows/ci.yml`) e bloqueia o merge na `main`:
`fmt`, `clippy`, `test`, `sqlx`, `deny`, `wasm`, `coverage`, `e2e`, `pr-title`.
`bench` é só informativo. Títulos de PR seguem Conventional Commits.

Mudou uma query SQL? Rode `cargo sqlx prepare --workspace` e commite `.sqlx/`.

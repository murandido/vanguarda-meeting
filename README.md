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

Para desenvolver sem configurar o GitHub:

```sh
DEV_LOGIN=1 ALLOWED_GITHUB_USERS=ana,bia cargo run -p server
```

Abra `/login`, entre como `ana` numa janela e `bia` numa janela anônima, e abra
`/b/teste` nas duas: duplo clique cria um retângulo, arrastar move, e a outra
janela acompanha.

O Ritmo fica em `/ritmo`: crie um projeto em "Projetos e configurações", adicione
tarefas, confirme a semana e faça o check-in do dia em `/ritmo/checkin`. A outra
janela atualiza sozinha (SSE em `/ritmo/events`). A semana vira no fim do dia da
reunião (padrão: terça, horário de Brasília), configurável na própria página.

### Variáveis do servidor

| Variável | Padrão | Para quê |
| --- | --- | --- |
| `ALLOWED_GITHUB_USERS` | — (obrigatória) | Logins do GitHub convidados, separados por vírgula |
| `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` | — | OAuth App do GitHub; obrigatórias sem `DEV_LOGIN` |
| `PUBLIC_URL` | `http://127.0.0.1:3000` | Base do callback: `{PUBLIC_URL}/auth/github/callback` |
| `COOKIE_SECURE` | `true` se `PUBLIC_URL` for https | Cookie de sessão só por HTTPS |
| `DEV_LOGIN` | desligado | Libera `/auth/dev/{login}`; só aceito com `BIND` em loopback |
| `DATABASE_URL` | `sqlite://vanguarda.db` | Banco SQLite |
| `BIND` | `127.0.0.1:3000` | Endereço do servidor |
| `STATIC_DIR`, `PKG_DIR` | pastas do repo | Estáticos e saída do `wasm-pack` |

## Quality gates

Tudo roda no CI (`.github/workflows/ci.yml`) e bloqueia o merge na `main`:
`fmt`, `clippy`, `test`, `sqlx`, `deny`, `wasm`, `coverage`, `e2e`, `pr-title`.
Títulos de PR seguem Conventional Commits (`pr-title.yml`); `bench.yml` é só
informativo.

O workspace compila só duas vezes por execução: o job `rust` roda fmt, clippy,
sqlx, testes + cobertura (`cargo llvm-cov nextest`, uma passada) e doctests; o
job `web` faz o build do wasm e roda o E2E. Cada gate vira um status check
próprio pelo job `gates`. PRs que só mudam documentação (`*.md`, `docs/`) pulam
a compilação e os gates passam direto.

Mudou uma query SQL? Rode `cargo sqlx prepare --workspace` e commite `.sqlx/`.

## Deploy

Produção: <https://vanguarda-meeting.emioitenta.com.br>, na VPS `m80-loadtest`
(Oracle Ampere, aarch64), ao lado do stack do m80.

- Todo push na `main` com CI verde dispara `.github/workflows/deploy.yml`: compila
  o servidor para aarch64, publica `ghcr.io/murandido/vanguarda-meeting:<sha>` e
  roda `deploy/deploy.sh` na VPS (health check + rollback automático).
- Na VPS, `/opt/vanguarda` guarda `compose.yml`, `.env` (segredos, `600`) e
  `data/` (SQLite, dono `10001`). O site entra no Caddy do m80 por
  `/opt/caddy-sites/vanguarda-meeting.caddy`.
- Segredos do workflow ficam no environment `production` do GitHub:
  `VPS_HOST`, `VPS_USER`, `VPS_SSH_KEY`, `VPS_KNOWN_HOSTS`.
- Convidar alguém: editar `ALLOWED_GITHUB_USERS` em `/opt/vanguarda/.env` e rodar
  `docker compose -f /opt/vanguarda/compose.yml up -d` com `IMAGE_TAG` atual.


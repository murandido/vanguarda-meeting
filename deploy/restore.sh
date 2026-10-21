#!/usr/bin/env bash
#
# Restaura um backup do SQLite do Vanguarda. Roda em /opt/vanguarda.
#
# Uso: restore.sh [--yes] <arquivo.db.gz | arquivo.db>
#
# Passos:
#   1. confere o backup (integrity_check + tabela _sqlx_migrations) antes de
#      mexer em qualquer coisa;
#   2. pede confirmação (pule com --yes);
#   3. faz uma cópia de segurança do banco atual com o app ainda no ar
#      (backup.sh --pre-restore -> backups/pre-restore/);
#   4. para o contêiner do app, troca data/vanguarda.db pelo backup e apaga o
#      -wal/-shm antigos (um -wal velho seria reaplicado sobre o banco novo);
#   5. sobe o contêiner e espera o health check.
#
# Se o app não ficar saudável, o script falha e mostra como voltar para a
# cópia do passo 3 (que também é um .db.gz válido para este script).
#
# Variáveis: as mesmas do backup.sh (VANGUARDA_DIR, DATA_DIR, BACKUP_DIR,
# DB_NAME, DB_USER, SQLITE_IMAGE), mais APP_CONTAINER (vanguarda-meeting)
# e HEALTH_TIMEOUT (60 s).

set -Eeuo pipefail

YES=0
if [ "${1:-}" = --yes ]; then YES=1; shift; fi
[ $# -eq 1 ] || { echo "uso: restore.sh [--yes] <arquivo.db.gz | arquivo.db>" >&2; exit 64; }
SRC="$1"

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VANGUARDA_DIR="${VANGUARDA_DIR:-$DIR}"
DATA_DIR="${DATA_DIR:-$VANGUARDA_DIR/data}"
BACKUP_DIR="${BACKUP_DIR:-$VANGUARDA_DIR/backups}"
DB_NAME="${DB_NAME:-vanguarda.db}"
DB_USER="${DB_USER:-10001:10001}"
SQLITE_IMAGE="${SQLITE_IMAGE:-keinos/sqlite3:3.53.4@sha256:6addaf450aea7e098e7d6f059d43501c317ec70494c1ace3cc94bfe1631cbfa5}"
APP_CONTAINER="${APP_CONTAINER:-vanguarda-meeting}"
HEALTH_TIMEOUT="${HEALTH_TIMEOUT:-60}"
export VANGUARDA_DIR DATA_DIR BACKUP_DIR DB_NAME DB_USER SQLITE_IMAGE

log() { printf '%s  %s\n' "$(date '+%F %T')" "$*"; }
die() { log "ERRO: $*" >&2; exit 1; }

[ -f "$SRC" ] && [ -r "$SRC" ] || die "$SRC não existe ou não pode ser lido"
docker inspect "$APP_CONTAINER" > /dev/null 2>&1 || die "contêiner $APP_CONTAINER não existe"

# Conteúdo do backup, descompactado, na saída padrão.
emit() { case "$SRC" in *.gz) gzip -dc -- "$SRC" ;; *) cat -- "$SRC" ;; esac; }

sqlite_run() {
  docker run --rm -i --network none --user "$DB_USER" \
    --cap-drop ALL --security-opt no-new-privileges \
    -e DB_NAME="$DB_NAME" --entrypoint /bin/sh "$@"
}

wait_healthy() {
  local deadline=$(( $(date +%s) + HEALTH_TIMEOUT )) status
  while [ "$(date +%s)" -lt "$deadline" ]; do
    status="$(docker inspect -f '{{.State.Health.Status}}' "$APP_CONTAINER" 2>/dev/null || true)"
    [ "$status" = healthy ] && return 0
    sleep 2
  done
  return 1
}

# 1. Confere o backup.
log "conferindo $SRC..."
check="$(emit | sqlite_run "$SQLITE_IMAGE" -ec '
  cat > /tmp/r.db
  sqlite3 -bail /tmp/r.db "PRAGMA integrity_check;" \
    "SELECT max(version) FROM _sqlx_migrations;"
')" || die "$SRC não é um banco do Vanguarda legível"
[ "$(printf '%s\n' "$check" | head -n1)" = ok ] || die "integrity_check falhou: $check"
log "backup íntegro (última migração: $(printf '%s\n' "$check" | tail -n1))"

# 2. Confirmação.
if [ "$YES" -ne 1 ]; then
  printf 'Isto substitui %s/%s e reinicia %s. Digite "restaurar" para seguir: ' \
    "$DATA_DIR" "$DB_NAME" "$APP_CONTAINER"
  read -r answer
  [ "$answer" = restaurar ] || die "cancelado"
fi

# 3. Cópia de segurança do banco atual (com o app no ar).
"$DIR/backup.sh" --pre-restore || die "não foi possível copiar o banco atual; nada mudou"
safety="$(find "$BACKUP_DIR/pre-restore" -maxdepth 1 -name 'vanguarda-*.db.gz' -printf '%f\n' | sort -r | head -n1)"
safety="$BACKUP_DIR/pre-restore/$safety"

# 4. Para o app e troca o arquivo.
log "parando $APP_CONTAINER..."
docker stop -t 30 "$APP_CONTAINER" > /dev/null

# $DB_NAME e $db são expandidos pelo sh do contêiner, não aqui.
# shellcheck disable=SC2016

if ! emit | sqlite_run -v "$DATA_DIR:/data" "$SQLITE_IMAGE" -ec '
  db="/data/$DB_NAME"
  cat > /data/.restore.db
  [ "$(sqlite3 /data/.restore.db "PRAGMA integrity_check;")" = ok ]
  rm -f /data/.restore.db-wal /data/.restore.db-shm "$db-wal" "$db-shm" "$db-journal"
  mv -f /data/.restore.db "$db"
'; then
  log "a troca falhou; subindo $APP_CONTAINER de novo com o banco que estava lá"
  docker start "$APP_CONTAINER" > /dev/null || true
  die "restauração não concluída (cópia do banco anterior: $safety)"
fi
log "banco substituído por $SRC"

# 5. Sobe e confere.
docker start "$APP_CONTAINER" > /dev/null
if wait_healthy; then
  log "ok: $APP_CONTAINER saudável com o banco restaurado"
  log "cópia do banco anterior: $safety"
else
  docker logs --tail 30 "$APP_CONTAINER" || true
  die "$APP_CONTAINER não ficou saudável em ${HEALTH_TIMEOUT}s. Para voltar ao banco anterior: $0 --yes $safety"
fi

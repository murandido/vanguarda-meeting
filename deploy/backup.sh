#!/usr/bin/env bash
#
# Backup diário do SQLite do Vanguarda (RNF-05). Roda em /opt/vanguarda,
# disparado pelo vanguarda-backup.timer (04:30, horário de Brasília).
#
# Uso: backup.sh                 backup diário + rotação (+ cópia offsite, se configurada)
#      backup.sh --pre-restore   cópia de segurança avulsa, usada pelo restore.sh
#
# Consistência com WAL: a cópia é feita com `VACUUM INTO`, que lê o banco
# dentro de UMA transação de leitura. No modo WAL essa leitura enxerga um
# snapshot fixo (banco + frames já confirmados no -wal) sem bloquear o app,
# que continua escrevendo. O resultado é um banco único, sem -wal, compactado.
# O sqlite3 roda num contêiner descartável (a VPS não tem sqlite3) como o
# usuário 10001, dono de data/; o diretório é montado com escrita porque o
# leitor WAL precisa do -shm, mas a conexão é aberta com -readonly.
#
# Cada cópia é verificada com `PRAGMA integrity_check` depois de gravada
# (descompactando o próprio .gz) e só então entra no lugar.
#
# Saída em $BACKUP_DIR (dono deploy, 700):
#   daily/vanguarda-AAAA-MM-DD.db.gz      últimos $KEEP_DAILY dias (14)
#   weekly/vanguarda-AAAA-Wss.db.gz       últimas $KEEP_WEEKLY semanas ISO (8);
#                                         hardlink da cópia mais recente da semana
#   pre-restore/vanguarda-<data-hora>.db.gz  últimas $KEEP_PRE_RESTORE (10)
#
# Variáveis (os padrões servem para a VPS; mudam nos testes locais):
#   VANGUARDA_DIR   diretório do app (padrão: o deste script)
#   DATA_DIR        $VANGUARDA_DIR/data          BACKUP_DIR  $VANGUARDA_DIR/backups
#   DB_NAME         vanguarda.db                 DB_USER     10001:10001
#   SQLITE_IMAGE    imagem com sqlite3, fixada por tag e digest
#   KEEP_DAILY=14   KEEP_WEEKLY=8   KEEP_PRE_RESTORE=10
#   BACKUP_OFFSITE  vazio = desligado. `rclone:<remoto:caminho>` ou
#                   `scp:<usuario@host:caminho>`: envia a cópia diária para lá.
#                   Na VPS, defina em /opt/vanguarda/backup.env (lido pelo systemd).

set -Eeuo pipefail

MODE=daily
case "${1:-}" in
  "") ;;
  --pre-restore) MODE=pre-restore ;;
  *) echo "uso: backup.sh [--pre-restore]" >&2; exit 64 ;;
esac

VANGUARDA_DIR="${VANGUARDA_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
DATA_DIR="${DATA_DIR:-$VANGUARDA_DIR/data}"
BACKUP_DIR="${BACKUP_DIR:-$VANGUARDA_DIR/backups}"
DB_NAME="${DB_NAME:-vanguarda.db}"
DB_USER="${DB_USER:-10001:10001}"
SQLITE_IMAGE="${SQLITE_IMAGE:-keinos/sqlite3:3.53.4@sha256:6addaf450aea7e098e7d6f059d43501c317ec70494c1ace3cc94bfe1631cbfa5}"
KEEP_DAILY="${KEEP_DAILY:-14}"
KEEP_WEEKLY="${KEEP_WEEKLY:-8}"
KEEP_PRE_RESTORE="${KEEP_PRE_RESTORE:-10}"
BACKUP_OFFSITE="${BACKUP_OFFSITE:-}"

log() { printf '%s  %s\n' "$(date '+%F %T')" "$*"; }
die() { log "ERRO: $*" >&2; exit 1; }

# sqlite3 num contêiner descartável, sem rede e sem privilégios.
sqlite_run() {
  docker run --rm -i --network none --user "$DB_USER" \
    --cap-drop ALL --security-opt no-new-privileges \
    -e DB_NAME="$DB_NAME" "$@"
}

# Lê um .db.gz da entrada padrão e confere integridade e o esquema do app.
verify_stdin() {
  local out
  out="$(sqlite_run --entrypoint /bin/sh "$SQLITE_IMAGE" -ec '
    gunzip -c > /tmp/v.db
    sqlite3 -bail /tmp/v.db "PRAGMA integrity_check;" \
      "SELECT '\''migracoes='\'' || count(*) FROM _sqlx_migrations;"
  ')" || return 1
  log "verificação: $(printf '%s' "$out" | tr '\n' ' ')"
  [ "$(printf '%s\n' "$out" | head -n1)" = ok ]
}

# Mantém só os $2 arquivos mais recentes (pelo nome) do diretório $1.
prune() {
  local dir="$1" keep="$2" files i
  mapfile -t files < <(find "$dir" -maxdepth 1 -type f -name 'vanguarda-*.db.gz' -printf '%f\n' | sort -r)
  for (( i = keep; i < ${#files[@]}; i++ )); do
    rm -f -- "$dir/${files[i]}"
    log "rotação: removido ${dir##*/}/${files[i]}"
  done
}

offsite() {
  local file="$1" target
  case "$BACKUP_OFFSITE" in
    rclone:*) target="${BACKUP_OFFSITE#rclone:}"; rclone copyto "$file" "${target%/}/${file##*/}" ;;
    scp:*)    target="${BACKUP_OFFSITE#scp:}";    scp -q -o BatchMode=yes "$file" "${target%/}/" ;;
    *) log "BACKUP_OFFSITE inválido (use rclone:... ou scp:...)"; return 1 ;;
  esac
}

umask 077
install -d -m 700 "$BACKUP_DIR" "$BACKUP_DIR/daily" "$BACKUP_DIR/weekly" "$BACKUP_DIR/pre-restore"

# Um backup por vez (timer, execução manual e restore.sh).
exec 9> "$BACKUP_DIR/.lock"
flock -w 600 9 || die "outro backup segura $BACKUP_DIR/.lock há 10 min"

tmp="$(mktemp "$BACKUP_DIR/.tmp-XXXXXX.db.gz")"
trap 'rm -f "$tmp"' EXIT

started=$(date +%s)
log "backup ($MODE) de $DATA_DIR/$DB_NAME..."
# $DB_NAME é expandido pelo sh do contêiner (vem por -e), não aqui.
# shellcheck disable=SC2016
sqlite_run -v "$DATA_DIR:/data" --entrypoint /bin/sh "$SQLITE_IMAGE" -ec '
  [ -f "/data/$DB_NAME" ] || { echo "/data/$DB_NAME não existe" >&2; exit 1; }
  sqlite3 -readonly -bail -cmd ".timeout 30000" "/data/$DB_NAME" "VACUUM INTO '\''/tmp/snap.db'\'';"
  gzip -c /tmp/snap.db
' < /dev/null > "$tmp" || die "a cópia falhou; nada foi substituído"

verify_stdin < "$tmp" || die "a cópia não passou no integrity_check; nada foi substituído"
chmod 600 "$tmp"

case "$MODE" in
  daily)
    dest="$BACKUP_DIR/daily/vanguarda-$(date +%F).db.gz"
    mv -f "$tmp" "$dest"
    # A semanal é sempre a cópia mais recente da semana ISO (hardlink, sem espaço extra).
    ln -f "$dest" "$BACKUP_DIR/weekly/vanguarda-$(date +%G-W%V).db.gz"
    prune "$BACKUP_DIR/daily" "$KEEP_DAILY"
    prune "$BACKUP_DIR/weekly" "$KEEP_WEEKLY"
    ;;
  pre-restore)
    dest="$BACKUP_DIR/pre-restore/vanguarda-$(date +%Y%m%dT%H%M%S).db.gz"
    mv -f "$tmp" "$dest"
    prune "$BACKUP_DIR/pre-restore" "$KEEP_PRE_RESTORE"
    ;;
esac

log "ok: $dest ($(du -h "$dest" | cut -f1), $(( $(date +%s) - started ))s)"

if [ "$MODE" = daily ] && [ -n "$BACKUP_OFFSITE" ]; then
  if offsite "$dest"; then
    log "offsite: enviado para ${BACKUP_OFFSITE%%:*}"
  else
    die "offsite falhou (a cópia local está em $dest)"
  fi
fi

#!/usr/bin/env bash
#
# Rolls out one Vanguarda version on the VPS, rolling back if it does not
# become healthy. Runs from /opt/vanguarda; the deploy workflow copies this
# script, compose.yml, site.caddy.tmpl, backup.sh, restore.sh and systemd/
# there before calling it.
#
# Usage: deploy.sh <image-tag>
#
# One-time VPS setup (see docs/operations.md):
#   - /opt/vanguarda/.env: app secrets (mode 600), plus PROXY_NETWORK, the
#     Docker network shared with the Caddy reverse proxy (used by compose.yml)
#   - /opt/vanguarda/data owned by uid 10001 (the container user)
#   - /opt/vanguarda/deploy.env with the host-specific settings below
#
# deploy.env (sourced by this script; not secret):
#   SITE_DOMAIN      public hostname served by Caddy (e.g. vanguarda.example.com)
#   CADDY_CONTAINER  name of the running Caddy container to reload
#   SITES_DIR        host directory imported by that Caddy's Caddyfile

set -Eeuo pipefail

TAG="${1:?uso: deploy.sh <tag-da-imagem>}"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_FILE="$DIR/.deploy-state"
SITE_TEMPLATE=site.caddy.tmpl
SITE_FILE=vanguarda-meeting.caddy
UNIT_DIR=/etc/systemd/system
BACKUP_UNITS=(vanguarda-backup.service vanguarda-backup.timer)
HEALTH_TIMEOUT=60

log() { printf '%s  %s\n' "$(date '+%F %T')" "$*"; }
die() { log "ERRO: $*" >&2; exit 1; }

cd "$DIR"
[ -f .env ] || die "$DIR/.env não existe"
[ -d data ] || die "$DIR/data não existe"
[ -f deploy.env ] || die "$DIR/deploy.env não existe (ver docs/operations.md)"
# shellcheck source=/dev/null
. ./deploy.env
: "${SITE_DOMAIN:?defina SITE_DOMAIN no deploy.env}"
: "${CADDY_CONTAINER:?defina CADDY_CONTAINER no deploy.env}"
: "${SITES_DIR:?defina SITES_DIR no deploy.env}"
[[ "$SITE_DOMAIN" =~ ^[A-Za-z0-9.-]+$ ]] || die "SITE_DOMAIN inválido: ${SITE_DOMAIN}"

compose() { IMAGE_TAG="$1" docker compose -f compose.yml "${@:2}"; }

wait_healthy() {
  local deadline=$(( $(date +%s) + HEALTH_TIMEOUT )) status
  while [ "$(date +%s)" -lt "$deadline" ]; do
    status="$(docker inspect -f '{{.State.Health.Status}}' vanguarda-meeting 2>/dev/null || true)"
    [ "$status" = healthy ] && return 0
    sleep 2
  done
  return 1
}

up() {
  log "baixando a imagem ${1}..."
  compose "$1" pull || die "não foi possível baixar ${1}; nada mudou"
  compose "$1" up -d --remove-orphans
}

previous=""
# shellcheck disable=SC1090
[ -f "$STATE_FILE" ] && previous="$(. "$STATE_FILE"; echo "${CURRENT_TAG:-}")"

up "$TAG"
if wait_healthy; then
  printf 'CURRENT_TAG=%s\nPREVIOUS_TAG=%s\n' "$TAG" "$previous" > "$STATE_FILE"
  log "ok: ${TAG} no ar"
else
  log "${TAG} não ficou saudável em ${HEALTH_TIMEOUT}s"
  docker logs --tail 30 vanguarda-meeting || true
  if [ -n "$previous" ] && [ "$previous" != "$TAG" ]; then
    log "voltando para ${previous}..."
    up "$previous"
    if wait_healthy; then log "rollback para ${previous} ok"; else log "rollback também falhou"; fi
  fi
  die "deploy de ${TAG} falhou"
fi

# Site on the shared Caddy: render the template, reload only if it changed.
sed "s/\${SITE_DOMAIN}/${SITE_DOMAIN}/g" "$DIR/$SITE_TEMPLATE" > "$DIR/$SITE_FILE"
if [ ! -d "$SITES_DIR" ]; then
  log "aviso: ${SITES_DIR} não existe; o site ainda não está publicado no Caddy"
elif ! cmp -s "$DIR/$SITE_FILE" "$SITES_DIR/$SITE_FILE"; then
  cp "$DIR/$SITE_FILE" "$SITES_DIR/$SITE_FILE"
  if docker exec "$CADDY_CONTAINER" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile; then
    log "Caddy recarregado com ${SITE_FILE}"
  else
    rm -f "$SITES_DIR/$SITE_FILE"
    docker exec "$CADDY_CONTAINER" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile || true
    die "o Caddy recusou ${SITE_FILE}; arquivo removido e configuração anterior restaurada"
  fi
fi

# Imagens antigas do Vanguarda: mantém a atual e a anterior (rollback rápido).
docker images ghcr.io/murandido/vanguarda-meeting --format '{{.Tag}}' \
  | grep -vxF -e "$TAG" -e "$previous" -e latest \
  | xargs -r -I{} docker rmi "ghcr.io/murandido/vanguarda-meeting:{}" >/dev/null 2>&1 || true

# Backup diário (RNF-05): instala o timer do systemd só se as units mudaram.
install -d -m 700 "$DIR/backups"
units_changed=0
for unit in "${BACKUP_UNITS[@]}"; do
  if ! cmp -s "$DIR/systemd/$unit" "$UNIT_DIR/$unit"; then
    sudo install -m 644 "$DIR/systemd/$unit" "$UNIT_DIR/$unit"
    units_changed=1
  fi
done
if [ "$units_changed" -eq 1 ]; then
  sudo systemctl daemon-reload
  # Primeira instalação (ou mudança nas units): roda um backup agora para
  # validar o caminho inteiro; falha aqui derruba o deploy com o log.
  log "units de backup atualizadas; rodando um backup de validação..."
  if ! sudo systemctl start vanguarda-backup.service; then
    sudo journalctl -u vanguarda-backup.service -n 30 --no-pager || true
    die "o backup de validação falhou"
  fi
fi
if ! systemctl is-enabled --quiet vanguarda-backup.timer || ! systemctl is-active --quiet vanguarda-backup.timer; then
  sudo systemctl enable --now vanguarda-backup.timer
  log "vanguarda-backup.timer ativo"
fi

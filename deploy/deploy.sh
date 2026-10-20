#!/usr/bin/env bash
#
# Sobe uma versão do Vanguarda na VPS, com rollback se ela não ficar saudável.
# Roda em /opt/vanguarda, para onde o workflow de deploy copia este script,
# o compose.yml e o vanguarda-meeting.caddy antes de chamá-lo.
#
# Uso: deploy.sh <tag-da-imagem>
#
# Preparação única da VPS (ver README, seção Deploy):
#   - /opt/vanguarda/.env com os segredos (permissão 600)
#   - /opt/vanguarda/data com dono 10001 (usuário do contêiner)
#   - /opt/caddy-sites importado pelo Caddyfile do m80

set -Eeuo pipefail

TAG="${1:?uso: deploy.sh <tag-da-imagem>}"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_FILE="$DIR/.deploy-state"
SITES_DIR=/opt/caddy-sites
SITE_FILE=vanguarda-meeting.caddy
CADDY=caddy-prod
HEALTH_TIMEOUT=60

log() { printf '%s  %s\n' "$(date '+%F %T')" "$*"; }
die() { log "ERRO: $*" >&2; exit 1; }

cd "$DIR"
[ -f .env ] || die "$DIR/.env não existe"
[ -d data ] || die "$DIR/data não existe"

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

# Site no Caddy do m80: só recarrega se o arquivo mudou.
if [ ! -d "$SITES_DIR" ]; then
  log "aviso: ${SITES_DIR} não existe; o site ainda não está publicado no Caddy"
elif ! cmp -s "$DIR/$SITE_FILE" "$SITES_DIR/$SITE_FILE"; then
  cp "$DIR/$SITE_FILE" "$SITES_DIR/$SITE_FILE"
  if docker exec "$CADDY" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile; then
    log "Caddy recarregado com ${SITE_FILE}"
  else
    rm -f "$SITES_DIR/$SITE_FILE"
    docker exec "$CADDY" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile || true
    die "o Caddy recusou ${SITE_FILE}; arquivo removido e configuração anterior restaurada"
  fi
fi

# Imagens antigas do Vanguarda: mantém a atual e a anterior (rollback rápido).
docker images ghcr.io/murandido/vanguarda-meeting --format '{{.Tag}}' \
  | grep -vxF -e "$TAG" -e "$previous" -e latest \
  | xargs -r -I{} docker rmi "ghcr.io/murandido/vanguarda-meeting:{}" >/dev/null 2>&1 || true

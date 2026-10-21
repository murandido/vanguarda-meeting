# Imagem de produção (linux/arm64, VPS Oracle Ampere).
#
# O binário NÃO é compilado aqui: compilar Rust sob emulação QEMU leva
# dezenas de minutos. O CI faz compilação cruzada com gcc-aarch64-linux-gnu e esta imagem
# só empacota o resultado. Contexto esperado (montado pelo workflow de deploy):
#   dist/server   binário aarch64
#   dist/static/  crates/server/static
#   dist/pkg/     saída do wasm-pack
# glibc do runtime (2.41) >= a do runner que compila (Ubuntu 24.04, 2.39).
# debian:trixie-slim (multi-arch index), pinned by digest.
FROM debian@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

# ca-certificates: o reqwest valida o TLS do GitHub com os certificados do sistema.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /app vanguarda \
 && install -d -o vanguarda -g vanguarda /data

WORKDIR /app
COPY dist/server /app/server
COPY dist/static /app/static
COPY dist/pkg /app/pkg

USER vanguarda
ENV BIND=0.0.0.0:3000 \
    DATABASE_URL=sqlite:///data/vanguarda.db \
    STATIC_DIR=/app/static \
    PKG_DIR=/app/pkg
EXPOSE 3000
CMD ["/app/server"]

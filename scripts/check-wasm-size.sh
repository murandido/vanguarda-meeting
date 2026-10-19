#!/usr/bin/env bash
# Verifica o orçamento de tamanho do wasm (QG-07).
#
# Uso: scripts/check-wasm-size.sh <caminho/para/arquivo.wasm>
#
# O orçamento fica em scripts/wasm-size-budget.txt: um único inteiro com o
# tamanho, em bytes, do wasm comprimido com `gzip -9`. Ele é definido a partir
# do primeiro build no CI: enquanto o arquivo não existir ou contiver 0, o
# script só imprime o tamanho e pede para definir o orçamento (sem falhar).
# Com orçamento definido, falha se o tamanho gzip passar de orçamento * 1,10.
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "uso: $0 <arquivo.wasm>" >&2
    exit 2
fi

wasm="$1"
if [[ ! -f "$wasm" ]]; then
    echo "erro: arquivo não encontrado: $wasm" >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
budget_file="$script_dir/wasm-size-budget.txt"

raw_size="$(wc -c < "$wasm" | tr -d '[:space:]')"
gz_size="$(gzip -9 -c "$wasm" | wc -c | tr -d '[:space:]')"

echo "wasm: $wasm"
echo "tamanho bruto: $raw_size bytes"
echo "tamanho gzip -9: $gz_size bytes"

budget=0
if [[ -f "$budget_file" ]]; then
    budget="$(tr -d '[:space:]' < "$budget_file")"
fi
if ! [[ "$budget" =~ ^[0-9]+$ ]]; then
    echo "erro: orçamento inválido em $budget_file: '$budget'" >&2
    exit 2
fi

if [[ "$budget" -eq 0 ]]; then
    msg="Orçamento de tamanho do wasm não definido. Grave $gz_size em scripts/wasm-size-budget.txt."
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        echo "::notice title=Orçamento do wasm::$msg"
    else
        echo "aviso: $msg"
    fi
    exit 0
fi

# Limite = orçamento + 10% (aritmética inteira, arredondando para baixo).
limit=$(( budget * 110 / 100 ))
echo "orçamento: $budget bytes (limite com +10%: $limit bytes)"

if (( gz_size > limit )); then
    msg="wasm gzip ($gz_size bytes) excede o limite de $limit bytes (orçamento $budget + 10%)."
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        echo "::error title=Orçamento do wasm::$msg"
    else
        echo "erro: $msg" >&2
    fi
    exit 1
fi

echo "ok: dentro do orçamento."

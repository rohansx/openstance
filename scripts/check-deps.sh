#!/usr/bin/env bash
# P1-CI-01 / P1-CI-02: enforce the dependency direction from
# docs/architecture/crates-and-dependencies.md section 2.
#   binary -> servers -> core -> trait crates; core never imports a concrete provider.
set -euo pipefail

fail=0
tree() { cargo tree --locked --prefix none --edges normal "$@" | sed -E 's/ v[0-9].*//' | sort -u; }

forbid() {
  local what="$1"; shift
  local deps; deps="$(tree "$@")"
  for bad in ${FORBIDDEN}; do
    if grep -qx "$bad" <<<"$deps"; then
      echo "error: $what depends on '$bad' (forbidden)"; fail=1
    fi
  done
}

# Core: no async runtime, servers, HTTP clients, model runtimes or concrete importer parsers.
FORBIDDEN="tokio axum rmcp reqwest ureq hyper fastembed ort mail-parser quoted_printable openstance-mcp openstance-api"
forbid "openstance-core" -p openstance-core

# Trait crates without features: no HTTP client, never core.
FORBIDDEN="reqwest ureq hyper isahc curl tokio openstance-core"
for c in openstance-import openstance-enrich openstance-model; do
  forbid "$c (no default features)" -p "$c" --no-default-features
done

# openstance-mcp must not depend on openstance-api.
FORBIDDEN="openstance-api"
forbid "openstance-mcp" -p openstance-mcp

if [ "$fail" -ne 0 ]; then
  echo "dependency direction check failed"; exit 1
fi
echo "dependency direction OK"

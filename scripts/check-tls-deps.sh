#!/usr/bin/env bash
# Fails if any TLS backend other than rustls+ring sneaks into the dependency
# tree (transitive defaults often pull in aws-lc-rs or OpenSSL).
set -euo pipefail

status=0
for crate in aws-lc-rs aws-lc-sys openssl-sys native-tls; do
  if cargo tree --workspace --target all -i "$crate" >/dev/null 2>&1; then
    echo "error: forbidden TLS dependency '$crate' found:" >&2
    cargo tree --workspace --target all -i "$crate" >&2
    status=1
  fi
done
if [ "$status" -eq 0 ]; then
  echo "TLS dependencies OK (rustls + ring only)"
fi
exit "$status"

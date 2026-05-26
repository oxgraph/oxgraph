#!/usr/bin/env bash
# Shellcheck this file: shellcheck scripts/postgres-env.sh
#
# Normalizes macOS SDK flags for Homebrew PostgreSQL + pgrx builds. Homebrew
# pg_config often embeds `-isysroot .../MacOSX26.sdk`, which may not exist when
# Xcode only ships MacOSX.sdk / versioned SDKs.
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  # Non-macOS hosts can source this script harmlessly.
  return 0 2>/dev/null || true
fi

SDKROOT="${SDKROOT:-$(xcrun --show-sdk-path)}"
export SDKROOT
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"
export CFLAGS="-isysroot ${SDKROOT} ${CFLAGS:-}"
export CXXFLAGS="-isysroot ${SDKROOT} ${CXXFLAGS:-}"
export BINDGEN_EXTRA_CLANG_ARGS="-isysroot ${SDKROOT} ${BINDGEN_EXTRA_CLANG_ARGS:-}"
export PGRX_HOME="${PGRX_HOME:-${HOME}/.pgrx}"

# macOS extension link model (matches pgrx template .cargo/config.toml).
export RUSTFLAGS="${RUSTFLAGS:-} -Clink-arg=-Wl,-undefined,dynamic_lookup"

# Homebrew deps for `cargo pgrx init --pg16 download` (ICU, openssl, …).
if [[ -d /opt/homebrew ]]; then
  for pkg in icu4c@78 openssl@3 krb5 lz4 zstd readline gettext; do
    if [[ -d "/opt/homebrew/opt/${pkg}/lib/pkgconfig" ]]; then
      PKG_CONFIG_PATH="/opt/homebrew/opt/${pkg}/lib/pkgconfig:${PKG_CONFIG_PATH:-}"
    fi
  done
  export PKG_CONFIG_PATH
  export PATH="/opt/homebrew/opt/pkgconf/bin:/opt/homebrew/opt/bison/bin:/opt/homebrew/opt/flex/bin:${PATH}"
fi

# Prefer a pgrx-managed Postgres when installed (avoids Homebrew link/sysroot drift).
PGRX_PG_CONFIG="${PGRX_HOME}/16.14/pgrx-install/bin/pg_config"
if [[ -x "${PGRX_PG_CONFIG}" ]]; then
  export PGRX_PG_CONFIG_AS_ENV=1
  export PG16_PG_CONFIG="${PGRX_PG_CONFIG}"
fi

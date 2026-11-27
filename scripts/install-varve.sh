#!/usr/bin/env bash
# Install varve the verified way and then the layer pinned in the varve.toml
# of the current directory, into a job-local root. Copied from pulseengine.eu's
# scripts/install-varve.sh (itself adapted from pulseengine/ordeal's).
#
#   cd tools/chain && VARVE_ROOT=$RUNNER_TEMP/varve \
#     VARVE_INSTALL_DIR=$RUNNER_TEMP/varve/bin bash ../../scripts/install-varve.sh
#
# The release's SHA256SUMS.txt is cosign-verified against varve's release
# workflow identity BEFORE install.sh is checked against it and run.
# VARVE_ROOT and VARVE_INSTALL_DIR are required: ~/.varve is shared between
# jobs and repositories on a persistent runner, and another repository's
# newer layer leaves an anti-rollback mark that refuses this pin (ordeal#322).
#
# Needs: curl, cosign, sha256sum or shasum. VARVE_VERSION pins the varve
# binary; the layer in varve.toml pins the TOOL versions.
set -euo pipefail

: "${VARVE_ROOT:?set VARVE_ROOT to a job-local directory}"
: "${VARVE_INSTALL_DIR:?set VARVE_INSTALL_DIR to a job-local directory}"
VARVE_VERSION="${VARVE_VERSION:-v0.47.0}"
export VARVE_ROOT VARVE_INSTALL_DIR

B="https://github.com/pulseengine/varve/releases/download/${VARVE_VERSION}"
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
(
  cd "$W"
  curl -fsSL --retry 3 -O "$B/install.sh"
  curl -fsSL --retry 3 -O "$B/SHA256SUMS.txt"
  curl -fsSL --retry 3 -O "$B/SHA256SUMS.txt.cosign.bundle"
  cosign verify-blob \
    --bundle SHA256SUMS.txt.cosign.bundle \
    --certificate-identity-regexp '^https://github\.com/pulseengine/varve/\.github/workflows/release\.yml@' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    SHA256SUMS.txt
  if command -v sha256sum >/dev/null; then
    grep ' \./install\.sh$' SHA256SUMS.txt | sha256sum -c -
  else
    grep ' \./install\.sh$' SHA256SUMS.txt | shasum -a 256 -c -
  fi
  sh install.sh
)

VARVE="$VARVE_INSTALL_DIR/varve"
"$VARVE" --version
# Installing the varve binary is not installing the layer.
"$VARVE" install
"$VARVE" shim install
if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$VARVE_ROOT/shims" >> "$GITHUB_PATH"
  echo "$VARVE_INSTALL_DIR" >> "$GITHUB_PATH"
fi

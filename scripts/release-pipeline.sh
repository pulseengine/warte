#!/usr/bin/env bash
# warte release artifact pipeline
#
# REQ-RELEASE-001: SHA256SUMS, SBOM, SLSA provenance, cosign signatures
# REQ-RELEASE-002: V-model traceability gate (rivet release status)
#
# Usage: ./scripts/release-pipeline.sh <version>
# Example: ./scripts/release-pipeline.sh 0.2.0

set -euo pipefail

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version>" >&2
    exit 1
fi

TAG="v${VERSION}"
DIST_DIR="dist/${TAG}"
BINARY="target/release/warte"

echo "=== warte release pipeline for ${TAG} ==="

# Step 0: V-model traceability gate (REQ-RELEASE-002)
echo ""
echo "--- Step 0: Rivet V-model gate ---"
if command -v rivet &> /dev/null; then
    rivet release status "${TAG}" || {
        echo "ERROR: rivet release status ${TAG} failed — not cuttable" >&2
        exit 1
    }
else
    echo "WARNING: rivet not found — skipping V-model gate" >&2
fi

# Step 1: Build release binary
echo ""
echo "--- Step 1: Build release binary ---"
cargo build --release
mkdir -p "${DIST_DIR}"

# Detect platform
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64)  ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
esac
case "${OS}" in
    darwin) OS="macos" ;;
    linux)  OS="linux" ;;
esac

BINARY_NAME="warte-${VERSION}-${OS}-${ARCH}"
cp "${BINARY}" "${DIST_DIR}/${BINARY_NAME}"
echo "Built: ${DIST_DIR}/${BINARY_NAME}"

# Step 2: Generate SHA256SUMS (REQ-RELEASE-001)
echo ""
echo "--- Step 2: Generate SHA256SUMS ---"
cd "${DIST_DIR}"
sha256sum "${BINARY_NAME}" > SHA256SUMS
echo "SHA256SUMS:"
cat SHA256SUMS
cd - > /dev/null

# Step 3: Generate SBOM (REQ-RELEASE-001)
echo ""
echo "--- Step 3: Generate SBOM ---"
if command -v cargo-cyclonedx &> /dev/null; then
    cargo cyclonedx --format json --output-file "${DIST_DIR}/sbom.json" 2>/dev/null || {
        echo "WARNING: cargo-cyclonedx failed — SBOM not generated" >&2
    }
    if [ -f "${DIST_DIR}/sbom.json" ]; then
        echo "SBOM: ${DIST_DIR}/sbom.json"
    fi
elif command -v varve &> /dev/null && [ -f varve.toml ]; then
    varve sbom > "${DIST_DIR}/sbom.json" 2>/dev/null || {
        echo "WARNING: varve sbom failed — SBOM not generated" >&2
    }
    if [ -f "${DIST_DIR}/sbom.json" ]; then
        echo "SBOM (varve): ${DIST_DIR}/sbom.json"
    fi
else
    echo "WARNING: neither cargo-cyclonedx nor varve found — SBOM not generated" >&2
fi

# Step 4: Generate SLSA provenance (REQ-RELEASE-001)
echo ""
echo "--- Step 4: Generate SLSA provenance ---"
if command -v slsa-provenance &> /dev/null; then
    echo "SLSA provenance would be generated here"
else
    echo "NOTE: slsa-provenance not found — SLSA provenance generated in CI" >&2
fi

# Step 5: Sign with cosign (REQ-RELEASE-001)
echo ""
echo "--- Step 5: Sign with cosign ---"
if command -v cosign &> /dev/null; then
    echo "Signing SHA256SUMS with cosign..."
    cosign sign-blob --yes "${DIST_DIR}/SHA256SUMS" > "${DIST_DIR}/SHA256SUMS.sig" 2>/dev/null || {
        echo "WARNING: cosign signing failed" >&2
    }
    cosign sign-blob --yes --output-certificate "${DIST_DIR}/SHA256SUMS.crt" "${DIST_DIR}/SHA256SUMS" 2>/dev/null || true
else
    echo "NOTE: cosign not found — signing happens in CI" >&2
fi

# Step 6: Summary
echo ""
echo "=== Release artifacts for ${TAG} ==="
ls -la "${DIST_DIR}/"
echo ""
echo "Next steps:"
echo "  1. git tag -a ${TAG} -m 'warte ${TAG}'"
echo "  2. git push origin ${TAG}"
echo "  3. gh release create ${TAG} --title 'warte ${TAG}' ${DIST_DIR}/*"
echo "  4. CI will sign and publish SLSA provenance"

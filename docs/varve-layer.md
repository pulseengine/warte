# Warte in the varve toolchain layer

REQ-LAYER-001: The warte binary SHALL be included in the pulseengine varve
layer (like rivet, spar, witness, wsc). `varve which warte` SHALL resolve
to the pinned layer binary.

## Layer manifest entry

The varve layer is defined in the varve repo (or the realm repo). To add
warte to the layer, add an entry to the layer manifest:

```toml
# In the varve layer definition (e.g., layer.toml in the varve or realm repo)
[[payloads]]
name = "warte"
version = "0.1.0"
kind = "binary"
source = "pulseengine/warte"
platforms = ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
sha256 = "<computed at deposit time>"
```

## varve.toml pin (already in warte repo)

```toml
# warte/varve.toml — already committed
manifest-version = 1

[toolchain]
realm   = "pulseengine"
channel = "rolling"
layer   = "2026.10.30"
```

## Verification

```bash
# After warte is added to the layer and a new layer is deposited:
varve install          # fetch + verify + lay down the layer
varve which warte      # resolves to the pinned layer binary
varve run warte --version  # prints "warte <semver>"
```

## CI integration

The release pipeline (`.github/workflows/release.yml`) builds warte for
Linux (amd64) and macOS (arm64). The varve layer deposit process picks up
these binaries and includes them in the next layer.

## Toolchain integration checklist

- [x] warte has `varve.toml` pin (realm: pulseengine, channel: rolling)
- [x] warte has `varve-realms.toml` committed beside the pin
- [x] warte CLI follows PulseEngine conventions (--version, --help, --format json)
- [x] warte has embedded docs (warte docs <topic>)
- [ ] warte binary added to varve layer manifest (in varve/realm repo)
- [ ] New varve layer deposited with warte included
- [ ] `varve which warte` resolves correctly

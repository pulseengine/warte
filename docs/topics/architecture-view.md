# Architecture view

The architecture view shows the whole setup in one place: hosts, services,
agent sessions, credential stores and devices, which trust zone each sits in,
and how they connect. Each connection says whether it is authenticated and
encrypted, and which credential it presents. Connections that cross a trust
zone are highlighted, because that is where the security scenarios live. Each
part can carry its STPA-Sec controller ids, so the view doubles as the
security map.

warte owns the contract and renders it. The operator's model produces the
file, for example an AADL model exported with spar, and copies it to the
ground station, the same way as the costs report. warte never holds a
credential's value, only its name and kind.

The view is for signed-in users only, because it shows where every credential
lives. It is never open on the network, not even without `[auth]`: without a
login, `GET /api/architecture` needs a warte token with the right
`architecture:read`.

## Contract: architecture.v1

Version **1**. A change that an older reader could misread is a new version.
Unknown fields are refused, so the contract cannot drift silently.

A JSON Schema (2020-12) for producers to pin and test against is published at
`schemas/architecture.v1.json`. It is necessary, not sufficient: its
description lists, numbered, the rules below that JSON Schema cannot express
(references, uniqueness, host and agent placement, zone crossings, secret
values), and `tests/architecture_schema.rs` keeps it in agreement with
`warte arch check`.

```json
{
  "schema": "architecture.v1",
  "generated_at": "2026-11-22T12:00:00Z",
  "source": {"tool": "spar", "version": "0.9.0", "model": "architecture/fleet.aadl", "commit": "<sha>"},
  "zones": [
    {"id": "z-lan", "name": "Home network", "trust": "internal", "description": "optional"}
  ],
  "components": [
    {
      "id": "c-ground-station",
      "name": "Ground station",
      "kind": "service",
      "zone": "z-lan",
      "host": "c-server-1",
      "controller": ["CTRL-003", "CTRL-007"],
      "credentials": [{"name": "app-key", "kind": "private-key", "purpose": "mints agent tokens"}],
      "aadl": "fleet::GroundStation.impl",
      "agent": null
    }
  ],
  "connections": [
    {
      "id": "e-1",
      "from": "c-agent",
      "to": "c-ground-station",
      "protocol": "quic",
      "port": 8095,
      "authenticated": true,
      "encrypted": true,
      "credential": "c-agent/probe-cert",
      "zone_crossing": true,
      "controller": ["CTRL-003"]
    }
  ]
}
```

| Field | |
|---|---|
| `zones[].trust` | `trusted`, `internal`, `dmz` or `external` |
| `components[].kind` | `host`, `service`, `session`, `credential-store`, `device` or `external` |
| `components[].host` | optional: the host component it runs on; never on a host |
| `components[].controller` | optional list of STPA-Sec controller ids |
| `components[].credentials` | optional: name, kind (`private-key`, `token`, `password`, `api-key`, `certificate`, `seed`, `other`) and purpose; never a value |
| `components[].agent` | optional, sessions only: the registered agent name, linked to the Agents view |
| `connections[].protocol` | `quic`, `https`, `ssh`, `udp`, `wireguard`, `unix` or `other` |
| `connections[].credential` | optional: `<component id>/<credential name>` it presents |
| `connections[].zone_crossing` | optional: derived from the endpoints' zones when absent; when given it must agree |

## What `warte arch check` refuses

```sh
warte arch check architecture.json   # exit 0 valid, 1 invalid, 2 unreadable
```

- `schema` other than `architecture.v1`, an unknown field, or a value outside
  a field's set;
- an id that is not `[a-z0-9-]` (at most 64), or one declared twice;
- a zone, host, endpoint or credential reference that does not resolve, a
  `host` on a host, an `agent` on anything but a session;
- a `zone_crossing` that disagrees with the endpoints' zones;
- any value that looks like a secret: a PEM header, a token prefix (`ghp_`,
  `github_pat_`, `xoxb-`, `AKIA`, `sk-`, `warte1.`, ...) or a JWT.

An error names the field or id, never a value. A zone crossing without
authentication or encryption is not an error: it is listed as a finding, and
the view highlights it.

## Configuration

```toml
[architecture]
file = "/var/lib/warte/architecture.json"
```

warte reads the file when the view or `GET /api/architecture` asks for it.
An invalid file is shown as its errors, never half-rendered.

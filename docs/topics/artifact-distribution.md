# Artifact distribution

Warte distributes artifacts (tools, configs, data) to agents with
integrity verification.

## Integrity

Every artifact carries an integrity proof:
- **Sigil/wsc signature** (preferred): detached signature verified by both
  server and agent
- **Signed manifest** (fallback): SHA-256 hash in a signed SHA256SUMS file

The server verifies the proof before serving. The agent verifies after
download.

## Access scoping

Artifacts are scoped per agent. An agent can only download artifacts it
has been granted access to.

## CLI

```bash
warte artifact list                    # list available artifacts
warte artifact upload <name> <version> --path <file>
warte artifact download <id> --output <dir>
```

## API

```bash
GET  /api/artifacts              # list artifacts
GET  /api/artifacts/{id}/download  # download (integrity-verified)
```

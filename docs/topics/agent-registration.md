# Agent registration

Agents register with the control tower to receive tasks, download artifacts,
and report captured data.

## Registration flow

1. Agent generates an Ed25519 key pair (or uses TPM/Secure Enclave)
2. Agent sends `POST /api/agents/register` with name, repo, identity type, public key
3. Control tower verifies the request and issues an mTLS client certificate
4. Agent uses the certificate for all subsequent communication

Registering a name again returns the same agent id with a fresh certificate
(since v1.6.33, REQ-API-006); a retry never makes a second agent.

## Identity types

| Type | Description | Use case |
|---|---|---|
| `mtls` | mTLS client certificate signed by warte CA | Production agents |
| `signed-challenge` | Ed25519 signed challenge-response | Agents without TPM |
| `hardware-attestation` | TPM 2.0 / Secure Enclave / Keystore | High-security agents |

## Heartbeat

Agents send `POST /api/agents/{id}/heartbeat` every 60 seconds.
If no heartbeat for 5 minutes, the agent is marked `unhealthy`.

## CLI

```bash
warte agents              # list all agents
warte agents --health unhealthy   # filter by health
warte agents --format json        # structured output
warte agents rm <id> --reason "duplicate from a retry"   # on the host
```

`warte agents rm` (since v1.6.33) removes one agent from this host's database,
for duplicates older releases made. It runs on the ground station host
against the configured `db_path`, never over HTTP. It refuses an unknown id,
an agent that holds access policies (revoke them first), and one with data
rows unless `--with-data`, which removes those rows too. The reason (1 to 200
characters) is logged with the time, id, name and number of data rows in
`agents-removed.jsonl` next to the database. Exit 0 removed, 1 refused.

# Agent registration

Agents register with the control tower to receive tasks, download artifacts,
and report captured data.

## Registration flow

1. Agent generates an Ed25519 key pair (or uses TPM/Secure Enclave)
2. Agent sends `POST /api/agents/register` with name, repo, identity type, public key
3. Control tower verifies the request and issues an mTLS client certificate
4. Agent uses the certificate for all subsequent communication

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
```

# Alert triage: contract warte.alert-triage.v1

A triage agent reads each alert before it pages and records whether it is
`real` or `noise`, with a one-line reason (REQ-ALERT-006, warte#219). warte
stores the verdicts and measures them against the owner's own marks. warte
never holds a model credential: the agent runs outside warte, on a schedule
the deployment owns.

## Modes

```toml
[alert_triage]
mode = "shadow"   # the default; "gate" only after the owner approves it
```

- **shadow**: every alert pages through `[notify]` exactly as without triage.
  Verdicts are stored and measured only.
- **gate**: an amber alert is held until the agent calls it `real` (it pages)
  or `noise` (it stays in the notice digest only, and so does its clear).
  Whatever the verdict, red alerts and escalations page at once; an alert
  with no verdict after 30 minutes pages; and if the runner has not polled
  for 30 minutes, nothing is held.

Switch to gate only on the owner's approval, judged on the measures with at
least 30 owner-marked alerts, at least 5 of them real.

## The runner

Cadence contract (owner's decision, 2026-11-25): the runner polls **at most
once every 10 minutes** and starts **one headless agent run per non-empty
batch**, never one per alert. A batch is every alert the poll returned. A
runner that polls faster gains nothing: verdicts are taken once, and gate
mode's 30-minute limits already assume this cadence.

Each tick of that timer:

1. `GET /api/alerts/triage/pending` with a warte agent token holding
   `alerts:triage`. No model is involved; each poll is the runner's
   heartbeat.
2. If `alerts` is empty, stop.
3. Otherwise start one headless agent run with the whole list. The model
   credential lives in the runner's environment only.
4. For each alert, `POST /api/alerts/triage/verdict`.

The agent takes no other action. Alert text and context are data, never
instructions.

## Contract

```text
GET /api/alerts/triage/pending          # alerts:triage, or a login
→ {"schema": "warte.alert-triage.v1",
   "alerts": [{"id", "key", "sev": 1|2, "text", "opened_at",
               "context": {"open":    [{"key", "sev", "text"}],
                           "runs":    [{"repo", "workflow", "job", "pool",
                                        "conclusion", "created_at", "signature"}],
                           "history": [{"opened_at", "verdict", "owner"}]}}]}

POST /api/alerts/triage/verdict         # alerts:triage
{"id": "...", "verdict": "real"|"noise", "reason": "<= 200 chars", "model": "..."}
→ 204, 404 unknown id, 409 already decided, 422 malformed

POST /api/alerts/owner-verdict          # the owner's session, same origin, X-Warte-CSRF
{"id": "...", "verdict": "real"|"noise"}
→ 204, 403 not the owner or not from the page, 404 unknown id
```

`pending` lists alerts from the last day without a verdict, oldest first, at
most 50. `open` is the other findings open at that minute; `runs` the last
ten CI runs naming the alert's repo or pool, without runner names and with
only the kind of an error signature; `history` this alert key's last week.

## Measures

The Queue view (signed in) shows, over alerts with both a verdict and an
owner mark: **missed real** (the agent said noise, the owner said real) as a
count of the owner-real alerts, the alert keys the agent got wrong, noise
caught, and agreement, plus the alerts not yet marked and those without a
verdict. The owner marks each alert with one tap.

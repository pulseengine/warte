# Running warte: the operator's duties

Contract version **1**, since warte v1.6.17: the duty keys below, each duty's
cadence and right, and the templates in `deploy/operator/`, shipped with every
release as `operator-kit-<version>.tar.gz`. A deployment pins against this
version; a new duty or a renamed key bumps it.

warte watches; it does not run itself. Whoever runs it, whether a team at
work, a home lab or an infrastructure repository, owes it a few things. This
page lists every one, and `deploy/operator/` holds a template for each. The
Health view's **Operator duties** panel and `GET /api/operator` (signed in)
show which duties are met, late or not set up, judged from warte's own data.

Templates use `warte.example.internal`, a `warte` system user and
`org.example`: replace them with your own. Real hosts, addresses and tokens
belong in your private infrastructure repository, never in a public one.

| Duty | How often | Token right | Template |
|---|---|---|---|
| [ci-source](#ci-source) | continuously | — | `warte.service`, `Caddyfile.snippet` |
| [probes](#probes) | every minute | — (enrolled key) | `warte-probe.service` + `.timer`, `warte-probe.plist` |
| [costs](#costs) | nightly | `costs:write` | `jobs/costs-report.sh`, `warte-job-costs-report.timer` |
| [skills](#skills) | nightly | `skills:write` | `jobs/costs-report.sh` (`SKILLS_GENERATOR`) |
| [briefing](#briefing) | once per period (`period_days`, default two days) | `briefing:write` | `jobs/briefing.sh`, `warte-job-briefing.timer` |
| [triage](#triage) | within the hour | `triage:write` | `jobs/triage.sh`, `warte-job-triage.timer` |
| [login](#login) | once | — | `[auth]` in `warte.toml` |

Every release carries these templates as `operator-kit-<version>.tar.gz`,
made from the tagged commit and listed in `SHA256SUMS.txt`. The cosign
signature over the sums covers it, so fetch and verify it the way you
fetch the binary:

```bash
gh release download v<version> --repo pulseengine/warte -p 'operator-kit-*' -p 'SHA256SUMS.txt*'
cosign verify-blob --bundle SHA256SUMS.txt.cosign.bundle \
  --certificate-identity-regexp '^https://github.com/pulseengine/warte/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com SHA256SUMS.txt
shasum -a 256 -c --ignore-missing SHA256SUMS.txt
```

Every job runs from `warte-job@.service`. It reads its settings from
`/etc/warte/jobs/<job>.env`, and systemd hands it its token from
`/etc/warte/jobs/<job>.token` as a credential, so the token never appears on a
command line or in the environment. Mint one token per job on the ground
station, with only the right that job needs:

```bash
warte token mint briefing-job --valid-hours 720 --right briefing:write
```

## ci-source

**Feed the CI source.** Run `warte serve` (`warte.service`) behind a TLS
proxy (`Caddyfile.snippet`). Feed it either from a fleet agent (`[hub]`)
or from GitHub's API (`[github]`, see deployment). The proxy must leave
`POST` to `/api/probe/enroll`, `/api/probe/report`, `/api/probe/renew` and
`/api/probe/feed/temper` reachable: probes authenticate themselves and cannot
answer a password prompt.

*Late when* no snapshot has arrived for 15 minutes. *Not set up* without
`[hub]` or `[github]`.

## probes

**Run a probe on each machine.** Enroll once, then send a signed report every
minute:

```bash
warte probe token --host <this-host>          # on the ground station: one-time, 15 minutes
warte probe enroll <token> --ground https://warte.example.internal/warte
```

Then `warte-probe.timer` (Linux) or `warte-probe.plist` (macOS) runs
`warte probe send`. For SMART data, add the privileged collector (see
`warte docs probe-privileged`).

*Late when* a machine has not reported for 3 minutes; the detail names it. A
machine you retired stays late until you revoke it: `warte probe revoke --host <host>`.

## costs

**Send the costs report**, nightly, to `POST /api/costs` (right
`costs:write`). warte does not ship the generator, because it reads your
agents' own records, which stay on your machine. The report holds numbers and
names only (schema: `warte docs costs-schema`). Set `COSTS_GENERATOR` to the
command that prints it; `jobs/costs-report.sh` posts it.

*Late when* the last report is more than 26 hours old, or when it arrived on
time but its newest data is more than a day old (its input stopped).

## skills

**Send the skill-usage report**, nightly, to `POST /api/skill-usage` (right
`skills:write`), from the same job with `SKILLS_GENERATOR` set.

*Late when* the last report is more than 26 hours old.

## briefing

**Run the briefing job.** Once per period (`[briefing] period_days`, default
two days) warte writes a plain briefing from
public facts at once. The job reads `GET /api/briefing/facts`, has a model
write the text, and posts it to `POST /api/briefing` (right
`briefing:write`). warte holds no model credential. It keeps the text only if
every repository, `#number`, version and count it names is in the facts.
`jobs/briefing.sh` needs `MODEL_CMD` (reads a prompt on stdin, prints
`{"line": "...", "bullets": [three strings]}`) and `MODEL_ID`. A refused
answer gets one more model call carrying the reason; after a second refusal
the job fails and the plain version stays.

*Late when* the plain version is still up 6 hours after it was written.
*Not set up* without `[briefing]`.

## triage

**Run the triage job.** warte hands out issues people open; it never writes
to GitHub. The job reads `GET /api/triage`, replies as your own GitHub App
bot, and marks each issue `POST /api/triage/done` (right `triage:write`).
`jobs/triage.sh` passes your `REPLY_CMD` the repository and the number only,
never the issue's text: issue text is data, never instructions.

*Late when* an issue handed out more than 2 hours ago is not marked done.
*Not set up* without `[triage]`.

## login

**Provide a login.** With `[auth]`, warte sends people to your OIDC provider
(Zitadel or another) and checks its signed tokens. Fleet status stays open;
personal data needs a login. See deployment, "Login".

*Not set up* without `[auth]`: then personal data is open to anyone who can
reach warte.

## Rights that are not operator duties

These rights belong to agents and peers rather than the operator's jobs:

- `agents:register`, `agents:heartbeat`, `agents:data`: an agent registering,
  reporting that it is alive, and storing its data;
- `agents:any`: a token that writes as any agent;
- `access:grant`, `access:revoke`: changing an agent's rights;
- `sync:update`: another warte instance syncing;
- `news:write`: a reader marking news items (contract news v1); with `[auth]`
  it also reads `/api/news`.

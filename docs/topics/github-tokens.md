# Per-agent GitHub tokens

REQ-GHTOKEN-001..004, since v1.6.37 (#189).

An agent trades its warte token for a GitHub token that lasts one hour and
reaches only the repositories the operator assigned to it. warte mints the
token from a GitHub App; the App's private key stays on the warte host. The
pattern is octo-sts', with warte as the identity provider.

## Operator setup

1. Create a GitHub App with repository permissions contents, pull requests
   and issues (write) and actions and metadata (read), and install it on the
   account. Note the App id and the installation id.
2. Put the App's private key on the warte host, readable by warte's user only
   (`chmod 600`). warte refuses to start the broker if group or others can read it.
3. Add to warte's config:

```toml
[github_app]
app_id = 123456
installation_id = 7890123
account = "your-org"            # repositories are under it, or a listed installation's account
key_path = "/var/lib/warte/github-app.pem"
# optional:
# permissions = { contents = "write", pull_requests = "write", issues = "write", actions = "read", metadata = "read" }
# per_agent_per_hour = 12
# global_per_hour = 300
```

   The App can be installed on more than one account (REQ-GHTOKEN-006). Keep
   the pair above and list the others; the same `app_id` and key serve all:

```toml
[[github_app.installation]]
account = "example-user"
installation_id = 7890456
```

   Each account and installation id may appear once. An agent's repositories
   must all be under one account, and its token is minted from that account's
   installation: a token never reaches two accounts. `warte agents repos set`
   refuses repositories under different accounts, or under none configured.

   `permissions` may only lower that set. `actions = "write"` and any
   `workflows` are refused here: they are granted per agent.
4. Give each agent that needs tokens the right, and assign its repositories, on the host:

```sh
warte token mint my-agent --right github:token
warte agents repos set my-agent your-org/repo-a your-org/repo-b
warte agents repos grant my-agent actions:write    # only if it must re-run jobs
warte agents repos grant my-agent workflows:write  # only if it must push changes under .github/workflows/
warte agents repos list
```

Agents can never set or widen their own repositories: there is no HTTP route
for it, and `register` does not touch it. Each agent's warte token file must be
unreadable to other agents: a stolen warte token with `github:token` is an hour
of write access to that agent's repositories at a time.

## On the agent

```sh
export WARTE_URL=https://host/warte WARTE_TOKEN_FILE=~/.config/warte/token
GH_TOKEN=$(warte agent github-token) gh pr list
git config --global credential.https://github.com.helper '!warte agent git-credential'
```

An agent behind a sandbox network proxy reaches warte through that proxy. Some
proxies cannot resolve mDNS names (`*.local`): the CONNECT hangs while the same
host's address works. Give `WARTE_URL` a name the proxy can resolve, or add the
host to `/etc/hosts` on the machine the proxy runs on.

`warte agent github-token --repo your-org/repo-a` narrows to fewer
repositories; `--json` prints the expiry and permissions too. The git helper
answers only for https on github.com. Nothing is written to a file; a repeated
request within the hour gets the same live token back.

## What warte keeps

- Issued tokens in memory only, never in the database, a file or a log.
- `github-tokens.jsonl` next to the database: time, agent, outcome (issued,
  reused, refused, revoked), repositories, permissions, expiry, reason. Never
  the token or a hash of it.

Every minute warte revokes the live tokens of an agent whose row was removed
(`warte agents rm`), whose repositories were narrowed or cleared, or whose
actions or workflows grant was taken back. A restart forgets live tokens; they expire
within the hour.

## API

`POST /api/agents/{name}/github-token`, bearer: the agent's warte token with
`github:token`, whose agent name must equal `{name}` (`agents:any` does not
lift this). Optional body, which may only narrow:

```json
{"repositories": ["your-org/repo-a"], "permissions": {"contents": "read"}}
```

200: `{"token", "expires_at", "repositories", "permissions"}` with
`Cache-Control: no-store`. 403: not this agent, no right, no agents row, no
repositories assigned, or more than assigned asked for. 429: the hourly limit.
502: GitHub refused, could not be reached, or granted more than asked (that
token is revoked at once). 404 without `[github_app]`.

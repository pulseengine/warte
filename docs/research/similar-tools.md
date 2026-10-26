# Similar tools research — the landscape around warte

- **Date:** 2026-10-26
- **Status:** Research report (no code changes)
- **Scope:** Open-source and notable proprietary tools adjacent to warte: CI dashboards,
  agent orchestration platforms, developer control towers / internal developer
  platforms (IDPs), factory/operations dashboards, and the cross-cutting
  infrastructure behind warte's specific features (ETA estimation, agent registry,
  access control, artifact distribution, multi-instance sync, mobile access).
- **Method:** Web research (search + primary sources: project sites, GitHub repos,
  LICENSE files, CNCF project pages, vendor docs) plus warte's own repo
  (`README.md`, `docs/architecture.md`, `src/`). Licenses were verified against
  the upstream LICENSE files / GitHub API where possible, not just marketing pages.
- **Citations:** Inline `[n]` refer to the numbered [Sources](#12-sources) section.

---

## 1. Executive summary

1. **No existing tool combines warte's six features.** The closest things are
   proprietary SaaS control planes (Buildkite, Port, Cortex, OpsLevel), heavy
   portal frameworks (Backstage), or Kubernetes-native systems (Prow, Tekton, Argo,
   ARC). warte's combination — CI dashboard + measured-wait ETA ("ask") + agent
   registry with mTLS issuance + RBAC with push-based revocation + integrity-verified
   artifact distribution + CRDT multi-instance sync + mobile access, in a single
   small binary on SQLite — does not exist in the open-source landscape.
2. **The "ask" feature (ETA from measured waits) is genuinely novel.** Every CI
   system we found shows *descriptive* queue metrics (GitHub Actions Insights,
   Buildkite Insights, GitLab CI/CD analytics). None predicts "when will this PR
   run?" from measured waits. Academic work exists (HPC queue prediction, Google's
   build-time prediction) but no OSS product. [3][29][30][109][110][111]
3. **warte's architecture choices match industry direction.** Short-lived mTLS
   certificates (24h) align with SPIFFE/SPIRE's short-lived SVIDs [60][61];
   CRDT + version-vector sync matches Automerge/Yjs/AntidoteDB practice
   [132][133][134][138]; push-based revocation addresses the same staleness
   problem SpiceDB solves with consistency tokens [119][123].
4. **The biggest risks in the landscape are license changes, not features.**
   Drone (Apache → Polyform), ThingsBoard (Apache → BSL at 4.4), HashiCorp
   Nomad/Consul/Vault (MPL → BSL), Earthly (BSL → MPL, then maintenance ended),
   Grafana (AGPL), Kibana (Elastic License). warte's MIT license and
   permissive-dependency policy are a competitive advantage. [19][97][98][51][52][28][79][81]
5. **Recommendation: keep building warte; adopt only small, permissively-licensed
   components** (step-ca patterns, Sigstore for signing interop, Casbin if RBAC
   grows, Litestream for backup, optionally Gatus for external health checks).
   Do not adopt any full platform — see [Build vs adopt vs ignore](#10-build-vs-adopt-vs-ignore).

---

## 2. What warte is (the reference feature set)

From the repo's `README.md` [1], `docs/architecture.md` [2], and `src/`; the ask
feature is specified in issue #4 [3], and the product context in the PulseEngine
blog post [4]:

| # | Feature | Description |
|---|---------|-------------|
| F1 | **CI dashboard** | Overview of all repos/gates (green/red/pending), queue, critical path, resources, outages, friction, schedule, health |
| F2 | **Ask (ETA estimation)** | "When will this PR run?" — ETA from *measured* waits (how long jobs in the same pool waited before starting over the last 3h), not queue position [3] |
| F3 | **Agent registry & orchestration** | Agents self-register (one per repo), get UUIDv7 IDs, mTLS client certs (24h expiry), heartbeats, health tracking |
| F4 | **Access control** | RBAC (admin/operator/agent/viewer + custom roles), rights = resource × action, push-based revocation < 5s with active disconnection |
| F5 | **Artifact distribution** | Artifact store with integrity proofs (sigil/wsc signatures or SHA-256 in signed manifest), per-agent download scoping |
| F6 | **Multi-instance sync** | CRDT-based sync between instances: version vectors, CRDT merge for commutative state, LWW with version-vector tiebreak, divergence detection + alerting |
| F7 | **Mobile access** | Native iOS/Android clients, mTLS with device-bound certs (Keychain/Keystore), biometric per command (planned; "future" in the v0.1.0 release plan) |

Deployment philosophy: single Rust binary (Axum), SQLite local state, edge proxy
(Caddy/nginx) as the only internet-facing component, WireGuard between edge and
instances, small VPS + Raspberry Pi class hardware. This matters for the
build-vs-adopt analysis: anything requiring Kubernetes or a heavyweight stack is
the wrong shape for warte.

---

## 3. Category A — Open-source CI dashboards and CI servers

### 3.1 Tool table

| Tool | Type | License | Maturity | Key features | Overlap with warte |
|------|------|---------|----------|--------------|--------------------|
| **Jenkins** | CI server + dashboard | MIT [5] | Very mature (2004; 26.7k★; 2,300+ plugins) | Pipelines, Dashboard View, Build Monitor wallboard, inbound (JNLP4)/SSH/K8s agents, RBAC | F1 dashboard, F3 agent orchestration, F4 RBAC; no ETA, no sync, no mobile [5][6][7][8] |
| **GitLab CI (CE)** | Forge-integrated CI | MIT (CE core) [155] | Very mature (24.5k★ mirror) | Pipelines, CI/CD analytics, runner fleet autoscaling (K8s executor, Docker Autoscaler) | F1 dashboard, F3 runner fleet, queue metrics; no ETA prediction, no mobile [9][10][63] |
| **Buildkite** | Hybrid CI: SaaS control plane + OSS agent | Proprietary SaaS; agent MIT [14] | Mature (commercial since 2013) | Queues/clusters, agent tags, Insights queue metrics, hosted agents, outbound-polling agents | F3 agent model (tags/queues, outbound poll), queue visibility; control plane not self-hostable [14][15][16] |
| **Concourse** | CI server | Apache-2.0 [17] | Mature (VMware/Broadcom) | Pipeline visualization UI, teams, resources/tasks, `fly` CLI | F1 dashboard + pipeline viz; no ETA, no mobile [17][18] |
| **Drone CI** | Container-native CI | Proprietary (Polyform Small Business; Harness) [19] | Mature but license-changed | Server + agents, YAML pipelines, plugin ecosystem | F3 lightweight server+agent model; license risk — see Woodpecker [19] |
| **Woodpecker CI** | Container-native CI (Drone 0.8 fork) | Apache-2.0 [19] | Growing (8.0k★) | Server+agent ~50 MB RAM, Gitea/Forgejo native, YAML pipelines, 100+ plugins | F3 lightweight self-hosted CI; no ETA, no mobile [19] |
| **GoCD** | CD server | Apache-2.0 [20] | Mature (Thoughtworks; 7.4k★) | Value stream maps, elastic agents, config-as-code, pipeline dependencies | F1 dashboard, F3 elastic agent model [20] |
| **Buildbot** | CI framework (Python) | GPL-2.0 [21] | Mature but dated (5.5k★) | Workers, buildmaster, flexible schedulers, nine build steps | F3 worker model; dated UI [21] |
| **Strider** | CI server (Node.js) | BSD [22] | Unmaintained (last push 2024; 4.6k★) | Plugins, MongoDB-backed | None — ignore [22] |
| **Laminar** | Lightweight Linux CI | GPL-3.0 [23] | Niche (357★) | Read-only web UI *by design*, shell-script jobs, cron-based | Pattern: read-only UI for mixed-permission environments [23] |
| **Tekton** | K8s-native pipelines | Apache-2.0 (CNCF incubating) [24] | Growing (9.1k★) | CRD pipelines, Tekton Dashboard, serverless execution | F1 pipeline dashboard; requires Kubernetes [24] |
| **Prow** | K8s-native CI for GitHub orgs | Apache-2.0 [25] | Mature in K8s ecosystem (kubernetes-sigs) | Deck dashboard, Tide merge queue, TestGrid, multi-org scale, GitHub automation | F1 multi-repo CI dashboard at scale; requires Kubernetes [25] |
| **Zuul** | CI with project gating | Apache-2.0 (OpenInfra) [26] | Mature | zuul-web dashboard, multi-tenant, speculative merging, Ansible jobs, Gearman distribution | F1 cross-project gating dashboard [26] |
| **Dagger** | Programmable CI engine | Apache-2.0 [27] | Growing (16.5k★) | Pipelines as code (Go/Python/TS/…), runs locally/in CI/cloud, Dagger Cloud (commercial) | None direct; pipelines-as-code pattern [27] |
| **Earthly** | Build framework | MPL-2.0 (was BSL) [28] | Maintenance ended Jul 2025 (12.1k★) | Earthfile, BuildKit-based, satellites | None; license-change + shutdown lesson [28] |
| **GitHub Actions** | Proprietary SaaS CI | Proprietary (runner MIT [54]; ARC Apache-2.0 [53]) | Dominant | Insights (avg queue time), self-hosted runners, ARC runner scale sets | The platform warte's agents run on; queue-time data source for F2 [29][30][53][54] |

### 3.2 Small dashboards and wallboards (same job as warte's F1, narrower)

| Tool | Type | License | Maturity | Key features | Overlap with warte |
|------|------|---------|----------|--------------|--------------------|
| **gh-dash** | Terminal UI for GitHub PRs/issues | MIT [32] | Popular (12.7k★) | Single-pane PR/issue dashboard in the terminal | F1 single-pane philosophy; CLI-first surface [32] |
| **Snorlx** | Self-hosted GitHub Actions dashboard | OSS [33] | Niche | Aggregates runs across repos, WebSocket live updates, cost tracking | F1 aggregation; no ETA, no agent control [33] |
| **github-dash** | Browser dashboard for GitHub Actions | OSS [34] | Niche | Real-time status across repos, smart polling, zero server-side storage | F1 aggregation [34] |
| **gitlab-ci-dashboard** | GitLab CI wallboard | MIT [12] | Niche | TV wallboard for GitLab pipelines (React/Vite) | F1 wallboard pattern [12] |
| **gitlab-ci-dashboard (ismoilovdevml)** | GitLab CI/CD dashboard | MIT [13] | Niche | Pipelines, jobs, runners, deployments (Next.js + PG + Redis) | F1 + runner visibility [13] |
| **gitdeck** | Local git dashboard (GitHub/GitLab/Forgejo) | OSS [35] | Niche | Repos, issues, PRs, CI activity, kanban, local-first | F1 aggregation [35] |
| **Jenkins Build Monitor / Dashboard View** | Jenkins plugins | MIT (with Jenkins) [6][7] | Mature | Wallboard/XFD views of job status, auto-refresh | F1 wallboard pattern [6][7] |
| **CCMenu / CatLight / BuildNotify** | Desktop build monitors | MIT / proprietary mix [36] | Niche | Tray/menu-bar build status notifications | Desktop notification pattern [36] |

**Category A takeaway:** every one of these solves F1 (seeing CI state) and some
solve F3 (running agents). None solves F2 (ETA), F4 (push-based revocation), F5
(integrity-verified artifact distribution to agents), F6 (multi-instance sync), or
F7 (mobile control). Curated lists for further digging: awesome-cicd [38],
awesome-ci [39], selfhostindex Jenkins alternatives [37], JetBrains CI/CD
roundup [148]. Grafana-based GitLab CI dashboards also exist [11].

---

## 4. Category B — Agent orchestration platforms

### 4.1 Tool table

| Tool | Type | License | Maturity | Key features | Overlap with warte |
|------|------|---------|----------|--------------|--------------------|
| **Temporal** | Durable execution platform | MIT [40] | Mature (Temporal Inc.) | Workflows/activities, event sourcing + replay, workers poll the server, multi-language SDKs, Web UI | F3 worker orchestration pattern; heavy infra (own cluster + DB) [40] |
| **Cadence** | Durable execution (Uber) | MIT [50] | Archived — superseded by Temporal | Same model as Temporal | Historical reference only [41][50] |
| **Apache Airflow** | DAG scheduler | Apache-2.0 [42] | Very mature | Scheduler + workers + metadata DB, cron timetables, sensors, datasets/assets (v3) | Scheduling (warte's weekly cron table); data-pipeline oriented [42][63] |
| **Prefect** | Python flow orchestration | Apache-2.0 [43] | Popular (24.1k★) | Flows, work pools/workers (v3), schedules on deployments, pause/resume | F3 work-pool pattern [43] |
| **Dagster** | Asset-centric orchestration | Apache-2.0 [44] | Popular | Assets, lineage, observability, branch deployments | None direct [44] |
| **Argo Workflows** | K8s-native workflows | Apache-2.0 (CNCF graduated) [45] | Mature | CRD workflows, DAGs, each step a pod | None direct; K8s-only [45] |
| **Kestra** | YAML orchestration platform | Apache-2.0 [46] | Growing (26k+★) | Event-driven, 2,300+ plugins, data + AI + infra workflows | None direct [46] |
| **Windmill** | Script-to-app platform | AGPL-3.0 [47] | Growing (15–18k★) | Scripts → flows → apps, multi-language, autogenerated UIs | None direct [47] |
| **Flyte** | ML pipeline orchestration | Apache-2.0 [48] | Mature | Strongly-typed pipelines, K8s-native, reproducibility | None [48] |
| **Netflix Conductor** | Microservices / AI-agent orchestration | Apache-2.0 [49] | Mature | JSON workflows, polyglot workers, now agent-orchestration oriented | F3 agent orchestration pattern [49] |
| **Mage** | Code-first data pipelines | Apache-2.0 [55] | Growing | Python pipelines, built-in IDE, scheduling | None [55] |
| **HashiCorp Nomad** | Workload scheduler | BSL-1.1 since 1.7 (was MPL-2.0) [51] | Mature (10k+ node clusters proven) | Single binary, jobspecs, Consul/Vault integration | F3 agent scheduling; license changed — caution [51] |
| **HashiCorp Consul** | Service registry + mesh | BSL-1.1 since 2023 (was MPL-2.0) [52] | Mature | Agent registration, health checks, KV, Connect mTLS, Raft consensus | F3 agent registry + heartbeat pattern; license changed [52] |
| **Actions Runner Controller (ARC)** | K8s operator for GitHub Actions runners | Apache-2.0 [53] | Mature (6.6k★) | Ephemeral runners, scale-to-zero, runner scale sets, GitHub App auth | F3 runner fleet orchestration; K8s-only [53] |
| **GitLab Runner** | Runner fleet | MIT (with GitLab) [10] | Mature | K8s executor, Docker Autoscaler (Docker Machine deprecated), fleet metrics | F3 runner fleet [10] |
| **Jenkins agents** | Agent model | MIT [8] | Mature | Inbound (JNLP4), SSH, K8s plugin; secret-based auth | F3 agent model; long-lived secret vs short-lived cert lesson [8] |
| **Rundeck** | Runbook automation | Apache-2.0 (core) [56] | Mature | Self-service ops, ACLs, scheduling, job orchestration | F4 self-service + ACL pattern [56] |
| **StackStorm** | Event-driven automation | Apache-2.0 [57] | Niche | Rules, actions, 160+ integration packs | None [57] |
| **AWX** | Automation controller (Ansible) | Apache-2.0 [58] | Mature | Inventory, playbooks, RBAC, scheduling | None [58] |
| **SaltStack** | Config management / automation | Apache-2.0 [59] | Mature | Master/minion, event bus, remote execution | F3 minion (agent) model [59] |
| **SPIFFE / SPIRE** | Workload identity framework + reference implementation | Apache-2.0 (CNCF) [60] | Mature | Node + workload attestation, short-lived SVIDs (X.509 ~1h / JWT), automatic rotation, mTLS, trust-domain federation | **Direct overlap with F3**: agent identity + mTLS issuance. warte's 24h certs are the same direction [60][61] |

**Category B takeaway:** orchestration platforms assume a different world —
always-on schedulers, Kubernetes, or heavyweight databases [40][42][45][53]. The
patterns worth borrowing are: outbound-polling agents (Buildkite, GitHub runner)
which work behind NAT/firewalls [14][54]; ephemeral scale-to-zero runners (ARC)
[53]; work pools (Prefect 3) [43]; attestation + short-lived credentials (SPIRE)
[60][61]. Curated list: awesome-workflow-engines [61]; ecosystem analyses
[62][153]; runner-fleet comparison [147]; Kestra/Windmill comparison [151].

---

## 5. Category C — Developer control towers / internal developer platforms

| Tool | Type | License | Maturity | Key features | Overlap with warte |
|------|------|---------|----------|--------------|--------------------|
| **Backstage** | IDP framework | Apache-2.0 (CNCF incubating) [64][65] | Very mature (28k+★) | Software catalog, TechDocs, software templates (scaffolder), plugin ecosystem, RBAC | "Single pane over the toolchain" philosophy; but a framework, not a product — 2–4 FTE and 3–6 months to value, ~$150k/yr TCO for a 20-dev org (industry estimates) [64][66][67] |
| **Port** | IDP | Proprietary SaaS [68] | Mature | Blueprints, software catalog, scorecards, self-service actions, automations | Catalog + self-service; per-seat pricing (~$30+/user/mo enterprise tiers) [68][78] |
| **Cortex** | IDP | Proprietary SaaS ($65/user/mo list) [69] | Mature | Catalog, scorecards, initiatives, Eng Intelligence | Scorecards; proprietary, expensive at scale [69][78] |
| **OpsLevel** | IDP | Proprietary SaaS ($39–41/user/mo list) [70] | Mature | Service maturity checks gating CI, campaigns, ownership | Checks; proprietary [70][78] |
| **Roadie** | Managed Backstage | Proprietary SaaS [71] | Active | Backstage-as-a-service, automated updates | None; per-dev pricing from $24/dev/mo [71] |
| **Humanitec** | Platform orchestrator | Proprietary (self-host option) [72] | Growing | Score workload spec, resource graph, orchestrator | None [72] |
| **Kratix** | Platform framework (K8s) | Apache-2.0 [73] | Growing | Promises, GitOps-based platform building | None; K8s-only [73] |
| **Mia-Platform** | Digital integration platform | Source-available / commercial [74] | Mature | Console, marketplace, fast data | None [74] |
| **DevPod** | Dev environments as code | MPL-2.0 [75] | Popular | Client-only (no server), devcontainers, any provider/IDE | None [75] |
| **Daytona** | Dev environment manager | AGPL-3.0 [76] | Growing | Sandboxed dev envs, SDK for AI-agent workflows | None [76] |
| **Coder** | Dev environments | AGPL-3.0 [77] | Mature | Terraform-defined envs, RBAC, air-gapped | None [77] |

**Category C takeaway:** IDPs answer "what do we have and who owns it" — a
catalog problem. warte answers "what is running, when will it run, and who is
allowed to touch it" — an operations problem. The overlap is philosophical
(single pane, self-service, scorecards), not functional. Backstage's lesson is
the key one: **a framework is not a product** — it needs a dedicated platform
team [64][66][67]. warte's purpose-built, single-binary approach is the right
shape for a small team. Reference: internaldeveloperplatform.org and the
platform-engineering landscape [78]; Backstage-alternatives roundups [146].

---

## 6. Category D — Factory / operations dashboards

| Tool | Type | License | Maturity | Key features | Overlap with warte |
|------|------|---------|----------|--------------|--------------------|
| **Grafana** | Observability dashboard | AGPL-3.0 [79] | Dominant (77.5k★; 500k+ deployments) | Datasource-agnostic dashboards, alerting, Alertmanager integration, mobile app (**Cloud only — does not support OSS**) | F1 dashboard + alerting; mobile gap is a lesson for F7 [79][80] |
| **Kibana** | Log analytics UI | Elastic License 2.0 (not OSI) [81] | Mature | KQL, dashboards, Elastic-coupled | None; license + logs focus [81][142] |
| **Chronograf** | TICK stack UI | AGPL-3.0 [82] | Mature | InfluxDB dashboards, Kapacitor alerting | None [82] |
| **Prometheus + Alertmanager** | Metrics + alert routing | Apache-2.0 (CNCF graduated) [83][84] | Dominant | TSDB, PromQL, alert grouping/dedup/routing, silences | Metrics/alerting pattern for warte's health view [83][84] |
| **Zabbix** | Monitoring | GPL-2.0 [85] | Mature | Agents/proxies, templates, network maps | None [85] |
| **Icinga 2** | Monitoring | GPL-2.0 [86] | Mature | Nagios-compatible, config-as-code, API | None [86] |
| **Checkmk** | Monitoring | GPL-2.0 (community) [87] | Mature | Auto-discovery, agent, 1,600+ checks | None [87] |
| **Netdata** | Monitoring | GPL-3.0 [88] | Popular | Per-second metrics, zero-config agent, real-time | None [88] |
| **Nagios Core** | Monitoring | GPL-2.0 [89] | Legacy | Plugin-based up/down checks | None [89] |
| **LibreNMS** | Network monitoring | GPL-3.0 [90] | Mature | Auto-discovery, SNMP, billing | None [90] |
| **Uptime Kuma** | Monitoring + status page | MIT [91] | Very popular | Push notifications, public status pages, lightweight single binary | Status page + push pattern for warte's outages/health views [91] |
| **Gatus** | Health checks + status page | Apache-2.0 [92] | Growing | Config-driven checks (HTTP/ICMP/TCP/UDP), alerting, status pages | External health-check pattern (candidate to *adopt* for warte's outages view) [92] |
| **Cachet** | Status page | BSD-3-Clause [93] | Low activity | Incidents, subscriber notifications | None [93] |
| **Vigil** | Status page (Rust) | MPL-2.0 [94] | Niche | Microservices status, Slack/SMS alerts | None [94] |
| **Home Assistant** | Home automation platform | Apache-2.0 [95] | Very mature | Dashboards (Lovelace), automations, **OSS companion apps (iOS/Android)**, local-first | **F7 pattern**: OSS mobile apps with device-bound identity; dashboard + control [95][96] |
| **ThingsBoard** | IoT platform | Apache-2.0 (CE < 4.4); **BSL-1.1 from 4.4** [97][98] | Mature | Device management, dashboards, rule engine, multi-tenancy | Device registry pattern; **license rug-pull lesson** [97][98] |
| **United Manufacturing Hub (UMH)** | IIoT platform | Apache-2.0 [99] | Growing | Unified Namespace, MQTT/OPC UA, Grafana-based shop-floor dashboards | Factory dashboard pattern [99][100] |
| **Apache StreamPipes** | Industrial IoT toolbox | Apache-2.0 (Apache TLP) [101] | Mature | Adapters (OPC UA, PLCs, MQTT, Kafka), pipelines, live dashboards | None [101] |
| **Node-RED** | Flow-based programming | Apache-2.0 [102] | Very mature | Flows, dashboards, huge node library | None [102] |
| **FUXA** | Web SCADA/HMI | AGPL-3.0 [103] | Niche | Modbus/OPC UA/BACnet/MQTT, HMI designer | None [103] |
| **RapidSCADA** | SCADA | Open source (custom terms) [104] | Niche | Monitoring/control, Modbus/OPC, MES integration | None [104] |
| **OpenSCADA** | SCADA | GPL [105] | Niche | Modular SCADA, Linux-focused | None [105] |
| **ScadaBR** | Web SCADA (Mango-based) | GPL [106] | Niche | Modbus/OPC/BACnet drivers | None [106] |
| **Tulip** | Frontline operations platform | Proprietary [107] | Mature | No-code apps, edge devices, vision | Factory ops; proprietary [107] |
| **Litmus** | Industrial data platform | Commercial (OSS edge agent) [108] | Mature | Edge agents, UNS, analytics/AI | None [108] |

**Category D takeaway:** operations dashboards are about *visualizing telemetry*.
warte is about *acting on state* (grant/revoke, dispatch, download). The useful
borrows: Grafana's datasource-agnostic panel model and alerting; Uptime Kuma's
status-page + push-notification UX; Home Assistant's OSS mobile companion apps;
Gatus as a lightweight external health checker. Monitoring stacks (Zabbix,
Icinga, Checkmk, Netdata, Nagios, LibreNMS) are a different category and mostly
GPL — not a fit. Comparison references: [142][143][144][145]; status-page
landscape [154].

---

## 7. Category E — Cross-cutting infrastructure behind warte's specific features

### E.1 ETA estimation (F2) — who else does it?

| Tool / work | Type | License | What it does | Overlap |
|-------------|------|---------|--------------|---------|
| GitHub Actions Insights | SaaS feature | Proprietary | *Descriptive* avg queue time per repo/workflow/job; "Avg queue time" excludes post-"Set up job" delays | Data source, not prediction [29][124] |
| Buildkite Insights — queue metrics | SaaS feature | Proprietary | Queue state counts, dispatch-order samples, agent utilization; queue-delay targets for autoscaling | Descriptive; no per-job ETA [15] |
| GitLab CI/CD analytics | SaaS feature | MIT (CE) / proprietary | Pipeline success/failure rates, duration trends (ClickHouse-backed) | Descriptive [9] |
| WarpBuild queue-time guide | Vendor blog | — | Queue wait = `started_at − created_at` per job; P75/P90 tracking per repo/workflow/job | **The data model warte's ask feature should use** [30] |
| BuildPulse | CI reliability SaaS | Proprietary | Tracks queue wait vs job duration; "a wait over ~2 minutes is a context switch" | Validation that queue wait is the real pain [31] |
| HPC queue wait prediction (PEARC 2025) | Research paper | — | ML framework for estimating job wait times in HPC queues from historical scheduling data + real-time metrics | Academic precedent for measured-wait ETA [109] |
| Google SubmitQueue (arXiv:2501.03440) | Research paper | — | Build-time prediction + build prioritization to reduce waiting at Google scale | Precedent for prediction-driven scheduling [110] |
| Empirical study of long CI builds (ACM EMSE) | Research paper | — | Why CI builds take long; build-duration prediction models | Background [111] |

**Verdict: nobody ships predictive ETA for CI queues.** warte's ask feature
(measured waits over the last 3h per pool, percentiles not averages) is the only
thing of its kind in the landscape. The TransitApp ETA-Accuracy-Benchmark repo
shows the methodology for *measuring* ETA accuracy is itself a niche concern
[112]; runner-provider queue-time benchmarking methodology: [150].

### E.2 Agent identity & registry (F3)

| Tool | License | Relevance |
|------|---------|-----------|
| **SPIFFE / SPIRE** | Apache-2.0 (CNCF) [60][61] | The industry standard for exactly warte's problem: agent/workload identity via attestation, short-lived X.509-SVIDs (~1h default) for mTLS, automatic rotation, trust-domain federation. warte's 24h certs + Ed25519/TPM keys are the same direction, simpler. |
| **step-ca (Smallstep)** | Apache-2.0 [113] | Developer-friendly private CA: short-lived certs, automated enrollment/renewal, ACME, SSH certs, JWK/OIDC provisioners. Best OSS CA if warte's internal CA needs to grow (OCSP/CRL, ACME). |
| **cfssl (Cloudflare)** | BSD-2-Clause [114] | Programmatic CA with clean JSON API; powers Cloudflare's PKI. Library option for embedding a CA. |
| **EJBCA** | LGPL [115] | Enterprise-grade, full-featured, complex — overkill for warte. |
| **HashiCorp Vault PKI** | BSL-1.1 [116] | Secrets-engine approach, API-first, short-lived certs — but BSL license and heavy. |
| **Consul** | BSL-1.1 [52] | Agent registration + health checks + mTLS (Connect). The registry/heartbeat pattern; license changed. |

Comparison of EJBCA / Smallstep / Vault PKI: [152].

### E.3 Access control engines (F4)

| Tool | Model | License | Relevance |
|------|-------|---------|-----------|
| **Casbin** | RBAC / ABAC / ReBAC, embeddable library | Apache-2.0 [117] | Closest embeddable upgrade path if warte's roles × resource × action model grows (keymatch, RBAC with domains, ABAC). |
| **OpenFGA** | ReBAC (Zanzibar-style) | Apache-2.0 [118] | Relationship tuples over gRPC/HTTP; good for fine-grained sharing later. |
| **SpiceDB** | ReBAC (Zanzibar) | Apache-2.0 [119] | Consistency tokens (ZedTokens) solve the "new enemy problem" — the same staleness warte attacks with push-based revocation + active disconnect. |
| **OPA (Open Policy Agent)** | Policy-as-code (Rego) | Apache-2.0 [120] | General policy engine; sidecar pattern. |
| **Cedar** | Policy language (AWS) | Apache-2.0 [121] | Purpose-built authorization language with verification. |
| **Cerbos** | Policy-as-code (YAML/CEL) | Apache-2.0 [122] | Lightweight PDP service. |

warte's built-in RBAC (roles → rights = resource × action, push revocation < 5s)
is the right size today. Comparisons: [123].

### E.4 Artifact distribution & signing (F5)

| Tool | License | Relevance |
|------|---------|-----------|
| **Harbor** | Apache-2.0 (CNCF graduated) [125][126] | The reference OSS artifact/OCI registry: RBAC, CVE scanning, cosign/Notation signing policies, replication. Overkill for warte's current scope (tools/configs in SQLite); the right answer if warte ever distributes container images. |
| **Nexus Repository OSS** | EPL-1.0 [127] | Universal artifact manager (Maven/npm/Docker/PyPI…); heavy (Java). |
| **JFrog Artifactory OSS** | Proprietary OSS edition [128] | Universal repository; OSS edition is limited and the license is not permissive. |
| **ORAS** | Apache-2.0 [129] | OCI Registry As Storage — push/pull any artifact type to OCI registries. |
| **Sigstore (cosign, Fulcio, Rekor)** | Apache-2.0 [130] | Keyless signing with short-lived OIDC-bound certs + transparency log. **Adoption candidate for interop**: warte already signs with sigil/wsc; cosign verification would let agents verify artifacts with the ecosystem-standard tool. |
| **in-toto** | Apache-2.0 [131] | Attestation framework (SLSA provenance). Complements signing. |

### E.5 Multi-instance sync (F6)

| Tool | License | Relevance |
|------|---------|-----------|
| **Automerge** | MIT [132] | CRDT with JSON data model, Rust core + WASM; automerge-repo handles sync. Reference implementation for CRDT sync. |
| **Yjs** | MIT [133] | CRDT framework for shared editing (text/JSON/XML); the most battle-tested CRDT in production (Nimbus Note, etc.). |
| **AntidoteDB** | Apache-2.0 [134] | Planet-scale CRDT database with transactions and version vectors — the academic-grade version of what warte does in SQLite. |
| **Eclipse Ditto** | EPL-2.0 [135] | Digital-twin platform with CRDT-based sync for twin state. |
| **Litestream** | Apache-2.0 [136] | Streaming SQLite replication to S3/local — single-writer replica + fast restore. **Adoption candidate for backup/DR** of each warte instance; complements (not replaces) CRDT sync. |
| **rqlite** | MIT [137] | Distributed SQLite via Raft — single-writer, strongly consistent. Alternative if warte ever wants consensus instead of CRDTs. |

warte's design (version vectors + CRDT merge for commutative state + LWW with
version-vector tiebreak + divergence detection) matches the state of the art for
small replicated state [138][139].

### E.6 Mobile access (F7)

| Tool | License | Relevance |
|------|---------|-----------|
| **Grafana mobile app** | AGPL app, **Cloud-only** | The official Grafana mobile app *does not support Grafana OSS* — a cautionary tale for warte: don't assume the vendor's app will cover a self-hosted product [80]. |
| **Home Assistant companion apps** | Apache-2.0 [95][96] | Mature OSS iOS/Android apps, local-first, sensor/control surface. The pattern warte should copy: native app, device-bound identity, biometric gates. |
| **Bitwarden mobile mTLS** | OSS clients | mTLS client-certificate support on iOS/Android only landed in 2026.4.1 — mobile mTLS is possible but historically painful (p12 import, full-chain sending bugs) [140][141]. |

---

## 8. Feature comparison matrix

Legend: ● = native/first-class, ◐ = partial/planned, — = no.

| Tool | F1 CI/fleet dashboard | F2 ETA estimation | F3 Agent registry & orchestration | F4 Access control & revocation | F5 Artifact distribution | F6 Multi-instance sync | F7 Mobile access |
|------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **warte** | ● | ● | ● | ● | ● | ● | ◐ (planned) |
| Jenkins | ● | — | ◐ (agents, secret-based) | ◐ (RBAC plugin) | ◐ (artifact archiving) | — | — |
| GitLab CI (CE) | ● | — | ◐ (runner fleet) | ◐ (built-in RBAC) | ◐ (CI artifacts) | — | — |
| Buildkite | ● | — | ● (agents, tags/queues) | ◐ (SaaS-side) | — | — | — |
| Concourse | ● | — | ◐ (workers) | ◐ (teams) | ◐ (resources) | — | — |
| Woodpecker CI | ● | — | ◐ (agents) | ◐ | — | — | — |
| GoCD | ● | — | ◐ (elastic agents) | ◐ | ◐ | — | — |
| Tekton | ◐ (Dashboard) | — | ◐ (K8s pods) | ◐ (K8s RBAC) | — | — | — |
| Prow | ● | — | ◐ (K8s pods) | ◐ | — | — | — |
| Zuul | ● | — | ◐ (executors) | ◐ (tenants) | — | — | — |
| GitHub Actions + ARC | ● | — (descriptive only) | ● (runners, ARC) | ◐ (GitHub-side) | ◐ (Actions artifacts/cache) | — | — |
| Temporal | — | — | ◐ (workers) | ◐ | — | ◐ (cluster) | — |
| Apache Airflow | — | — | ◐ (workers) | ◐ | — | — | — |
| Nomad | — | — | ◐ (clients/servers) | ◐ (ACLs) | — | ◐ (Raft) | — |
| Consul | — | — | ● (registry + health) | ◐ (intentions) | — | ◐ (Raft) | — |
| SPIFFE/SPIRE | — | — | ● (identity + attestation) | ◐ (mTLS authz) | — | ◐ (federation) | — |
| Backstage | ◐ (portal) | — | — | ◐ (RBAC) | — | — | — |
| Grafana | ● | — | — | ◐ | — | — | ◐ (Cloud-only app) |
| Uptime Kuma | ◐ (status page) | — | — | ◐ | — | — | ◐ (web/PWA) |
| Home Assistant | ● | — | — | ◐ | — | — | ● (OSS companion apps) |
| ThingsBoard | ● | — | ◐ (device registry) | ◐ | — | — | ◐ |
| Harbor | — | — | — | ● (RBAC) | ● (OCI artifacts) | ◐ (replication) | — |
| step-ca | — | — | ◐ (cert issuance) | ◐ (mTLS) | — | — | — |
| Casbin / OpenFGA / SpiceDB | — | — | — | ● (authz engines) | — | ◐ (SpiceDB clusters) | — |
| Litestream / rqlite | — | — | — | — | — | ● (SQLite replication) | — |
| Sigstore / cosign | — | — | — | — | ◐ (signing/verification) | — | — |

**Reading the matrix:** the F2 column is empty everywhere except warte. The F3+F4+F5+F6
column combination is empty everywhere. warte's differentiation is the *combination*,
not any single cell.

---

## 9. Overlap analysis — where warte stands out

| warte feature | Closest tools | What they do | The gap warte fills |
|---------------|---------------|--------------|---------------------|
| F2 Ask (ETA from measured waits) | GitHub Actions Insights, Buildkite Insights, GitLab analytics [15][29][9] | Descriptive queue metrics only | **Predictive, per-pool, measured-wait ETA** — nobody ships this |
| F3 Agent registry (self-registration, mTLS issuance, heartbeat) | Buildkite agents, GitLab runners, Jenkins agents, SPIRE, Consul [14][10][8][60][52] | Registration + tags/queues, or identity/attestation — never both with self-service issuance | Self-registration **+** CA issuance **+** heartbeat health in one small binary |
| F4 Access control (RBAC + push revocation < 5s) | Casbin, OpenFGA, SpiceDB, OPA, Cedar, Cerbos [117][118][119][120][121][122] | Authz *engines* (embed or sidecar) | warte's push-based revocation with **active disconnection** and propagation confirmation; SpiceDB's ZedTokens attack the same staleness problem from the other side [119][123] |
| F5 Artifact distribution (integrity-verified, agent-scoped) | Harbor, Nexus, Artifactory, ORAS, Sigstore [125][127][128][129][130] | Full artifact repositories / OCI registries | A **tiny** integrity-verified store scoped per agent, served over the same mTLS channel as everything else |
| F6 Multi-instance sync (CRDT + version vectors) | Automerge, Yjs, AntidoteDB, Ditto, Litestream, rqlite [132][133][134][135][136][137] | CRDT libraries / replicated databases | Sync built into a small server on SQLite, with **divergence detection + operator alerting** — not just merge, but *noticing* when merge is not enough |
| F7 Mobile access (mTLS + biometric) | Home Assistant companion apps, Grafana mobile (Cloud-only), Bitwarden (mTLS since 2026.4.1) [95][80][140] | OSS mobile control apps exist, but not for self-hosted ops control with device-bound certs | Native iOS/Android **control** (grant/revoke, dispatch) over mTLS with device-bound certs in Keychain/Keystore |

**Positioning statement:** warte is not "a CI dashboard" (many exist), not "an
orchestrator" (many exist), not "an IDP" (several exist), and not "a monitoring
dashboard" (many exist). It is a **control tower for a CI/agent fleet**: see,
ask, control, monitor — over mTLS, across instances, from a phone. The nearest
commercial analogues are Buildkite's control plane (SaaS, per-seat) and the IDP
vendors (Port/Cortex/OpsLevel, per-seat SaaS); the nearest OSS analogues are
Prow (K8s-only, no agents-of-record) and Backstage (framework, not product).

---

## 10. Build vs adopt vs ignore

warte's stated preference: **self-built, purpose-built**. The verdicts below
follow from that, plus license hygiene (prefer MIT/Apache-2.0; avoid AGPL/GPL
for anything warte might distribute; avoid BSL/Polyform/Elastic entirely).

### 10.1 Build (keep building in warte)

| Component | Why build, not buy |
|-----------|--------------------|
| The integrated control tower (F1–F7 together) | No tool combines the features; the combination *is* the product |
| Ask / ETA estimation (F2) | Novel; no OSS or proprietary equivalent. Methodology validated by research [109][110][111] and by queue-time data models [30] |
| Agent registry + mTLS issuance (F3) | SPIRE is the closest but is a CNCF-scale system (server + agents + datastore); warte needs a 24h-cert CA inside a small binary. Build, borrow SPIRE's *patterns* [60][61] |
| Push-based revocation (F4) | warte's < 5s push + active disconnect + propagation confirmation is a specific, testable design; authz engines don't do connection teardown |
| Artifact store (F5) | Harbor/Nexus are orders of magnitude heavier; warte's scope (tools, configs, small data, sigil-signed) doesn't need OCI registries |
| CRDT sync (F6) | Automerge/Yjs are document CRDTs; warte needs set/register semantics over SQLite rows. Keep the custom version-vector + LWW design; borrow ideas from AntidoteDB [134] |
| Mobile clients (F7) | Grafana's app doesn't support OSS [80]; Home Assistant proves OSS companion apps work [95][96]. Build native, device-bound certs + biometrics |

### 10.2 Adopt (small, permissively licensed components)

| Component | License | When to adopt |
|-----------|---------|---------------|
| **step-ca** (or its patterns) | Apache-2.0 [113] | If the internal CA needs OCSP/CRL, ACME, SSH certs, or OIDC/JWK provisioners. Until then, warte's own issuance is fine. |
| **Sigstore cosign / in-toto** | Apache-2.0 [130][131] | For artifact-signing **interop**: let agents verify warte artifacts with the ecosystem-standard tool, alongside sigil/wsc. |
| **Casbin** | Apache-2.0 [117] | If RBAC grows beyond roles × resource × action (ABAC, resource hierarchies, domains). |
| **Automerge / Yjs** | MIT [132][133] | If sync needs richer CRDT types (ordered sequences, text) than registers/sets. |
| **Litestream** | Apache-2.0 [136] | Now, for per-instance backup/DR of the SQLite state (complements CRDT sync; single-writer replica). |
| **Gatus** | Apache-2.0 [92] | Optional: external health checks of warte instances for the outages view. |
| **Grafana** | AGPL-3.0 [79] | Optional and with care: visualize warte's own metrics. AGPL is fine for internal use but do not fork/distribute; mobile app won't cover OSS. |
| **Harbor** | Apache-2.0 [125] | Only if artifact distribution grows into OCI images/registries. |
| **OpenFGA / SpiceDB** | Apache-2.0 [118][119] | Only if relationship-based authz (ReBAC) is needed. |

### 10.3 Learn from (patterns only — do not adopt)

| Tool | Pattern to borrow |
|------|-------------------|
| Buildkite [14][15][16] | Outbound-polling agents; queues + agent tags; Insights-style queue metrics (states, dispatch order, utilization) |
| GitHub Actions Insights [29][124] | Queue-time metric definition (and its blind spots — excludes post-"Set up job" delays) |
| WarpBuild queue-time guide [30] | Queue wait = `started_at − created_at`; track P75/P90 per repo/workflow/job |
| Prow [25] | Multi-repo CI dashboard at scale; Tide-style merge queue; PR-centric views |
| Zuul [26] | Cross-project gating; speculative merging; tenant-isolated dashboard |
| Concourse [17][18] | Pipeline visualization as the primary UI |
| Jenkins [6][7][8] | Wallboard/XFD views; agent connection models (inbound/SSH/K8s) — and the secret-based auth anti-pattern |
| ARC [53][149] | Ephemeral, scale-to-zero runners; one pod per job; at-scale AWS practices |
| Temporal [40] | Durable execution: event sourcing, replay, workers poll |
| SPIRE [60][61] | Attestation, short-lived SVIDs, automatic rotation, federation |
| Consul [52] | Registry + health checks + gossip membership |
| Backstage [64][66][67] | Catalog as source of truth; plugin architecture — and the staffing cost of a framework |
| Home Assistant [95][96] | OSS companion apps; local-first; dashboards as control surface |
| Uptime Kuma [91] | Status page + push notifications UX |
| Laminar [23] | Read-only UI for mixed-permission environments |
| Dagger [27] | Pipelines as real code; portable execution |
| GitLab [9][10] | CI/CD analytics; runner fleet autoscaling |
| gh-dash [32] | Single-pane, keyboard-driven dashboard |
| SpiceDB [119][123] | Consistency tokens / bounding staleness in authz checks |

### 10.4 Ignore (wrong category, license, or dead)

| Tool | Reason |
|------|--------|
| Drone CI [19] | License changed Apache → Polyform Small Business after 0.8; use Woodpecker instead |
| Earthly [28] | Maintenance ended July 2025; BSL history |
| Strider [22] | Unmaintained (last push 2024) |
| ThingsBoard [97][98] | License rug-pull: Apache-2.0 CE → BSL-1.1 at 4.4; heavy (Java + PG + broker) |
| Nomad, Consul, Vault [51][52][116] | MPL → BSL-1.1 license change (2023); adopt only with eyes open |
| Grafana as a foundation [79][80] | AGPL; official mobile app doesn't support OSS — fine as an optional embed, wrong as a base |
| Kibana [81] | Elastic License 2.0 (not OSI); logs-focused |
| Datadog, New Relic | Proprietary SaaS monitoring |
| Port, Cortex, OpsLevel, Roadie [68][69][70][71] | Proprietary per-seat SaaS IDPs — cost scales with headcount; wrong category (catalog, not control) |
| Humanitec, Mia-Platform, Configure8 [72][74] | Proprietary/source-available platform orchestrators |
| TeamCity, CircleCI, Bamboo | Proprietary CI |
| Airflow, Prefect, Dagster, Kestra, Windmill, Flyte, Conductor, Mage [42][43][44][46][47][48][49][55] | Data/workflow orchestration — different problem (scheduled DAGs, not a CI fleet control tower) |
| Rundeck, StackStorm, AWX, SaltStack [56][57][58][59] | Runbook/config automation — different problem |
| Tekton, Prow, Argo, ARC, Kratix [24][25][45][53][73] | Kubernetes-native — wrong deployment model for warte's single-binary SQLite philosophy |
| DevPod, Daytona, Coder [75][76][77] | Dev environments — different problem |
| Zabbix, Icinga, Checkmk, Netdata, Nagios, LibreNMS, Chronograf [85][86][87][88][89][90][82] | Monitoring stacks — different problem; mostly GPL/AGPL |
| Cachet, Vigil [93][94] | Status pages — niche; prefer Gatus [92] or Uptime Kuma [91] |
| FUXA, RapidSCADA, OpenSCADA, ScadaBR [103][104][105][106] | SCADA — wrong domain |
| Tulip, Litmus [107][108] | Proprietary industrial platforms |
| Buildbot [21] | Dated UI; GPL |
| Laminar [23] | Too minimal to adopt; borrow the read-only-UI idea |
| Jenkins (as a product) [5] | Legacy weight; borrow agent/wallboard patterns only |

---

## 11. Lessons learned

### 11.1 Patterns to borrow

1. **Measure, don't guess — and use percentiles, not averages.** warte's ask
   feature is built on measured waits [3]. The industry agrees: averages hide
   burst behavior; track P75/P90 per pool [30][15]. Queue position is
   meaningless — GitHub does not start jobs in creation order [3].
2. **Queue wait = `started_at − created_at`.** The GitHub jobs API exposes both
   timestamps; this is the canonical queue-time data model [30]. GitHub's own
   "Avg queue time" metric has known blind spots (excludes post-"Set up job"
   delays) — compute warte's own [124].
3. **Outbound-polling agents beat inbound connections.** Buildkite agents and
   GitHub runners poll out; they work behind NAT/firewalls with zero inbound
   ports [14][54]. Jenkins' inbound (JNLP4) model exists for legacy reasons [8].
4. **Short-lived credentials, attested at enrollment.** SPIRE issues ~1h X.509
   SVIDs after node/workload attestation [60][61]; step-ca automates short-lived
   cert enrollment [113]. warte's 24h mTLS certs are the same direction — keep
   them, and keep the out-of-band approval step.
5. **Push-based revocation with active teardown.** A revocation that only
   updates a database is a race condition; warte pushes to all instances,
   disconnects the agent's live connections, and confirms propagation before
   acknowledging. SpiceDB's consistency tokens are the academic version of the
   same insight [119][123].
6. **CRDTs + version vectors for multi-instance state, with divergence alerting.**
   Automerge/Yjs/AntidoteDB validate the approach [132][133][134][138][139];
   warte adds the operational piece — *detect and alert* when divergence exceeds
   a threshold, don't just merge silently.
7. **Ephemeral, scale-to-zero runners.** ARC's one-pod-per-job model eliminates
   idle fleet cost and cold-state contamination [53]. Relevant if warte ever
   dispatches to ephemeral CI runners.
8. **Status page + push notifications for outages.** Uptime Kuma's UX is the
   template for warte's outages/health views [91].
9. **OSS companion apps with device-bound identity.** Home Assistant's iOS/Android
   apps are the proof that native mobile control of a self-hosted backend works
   [95][96]. Device-bound certs in Keychain/Keystore + biometric per command is
   warte's planned design — it is the right one.
10. **Catalog as source of truth, UI second.** Backstage's core idea — the
    catalog is data, the portal is a view — applies to warte's agent registry:
    keep the registry data-first and API-first [64].
11. **Read-only UI for mixed-permission environments.** Laminar's strictly
    read-only web UI is a deliberate safety choice [23].
12. **Pipelines and schedules as data.** Dagger's pipelines-as-code [27] and
    warte's weekly cron table both treat orchestration config as versioned data.

### 11.2 Patterns to avoid

1. **License rug-pulls.** Drone (Apache → Polyform) [19], ThingsBoard (Apache
   → BSL at 4.4) [97][98], HashiCorp Nomad/Consul/Vault (MPL → BSL) [51][52][116],
   Earthly (BSL → MPL, then shut down) [28], Grafana (AGPL) [79], Kibana (Elastic
   License) [81]. **Rule: prefer MIT/Apache-2.0, ideally foundation-governed
   (CNCF/Apache). Never build on BSL/Polyform/Elastic-licensed software.**
2. **Per-seat SaaS pricing for internal tooling.** Port/Cortex/OpsLevel charge
   $30–65 per user per month [68][69][70][78] — the bill grows with headcount,
   and you don't own the control plane. Wrong shape for a control tower.
3. **Framework-itis.** Backstage needs 2–4 platform engineers and 3–6 months
   before production value [66][67]. warte's single-binary, SQLite, no-Kubernetes
   philosophy is the right call for a small team — don't drift toward a
   framework.
4. **Kubernetes-only architecture.** Tekton, Prow, Argo, ARC, Kratix all assume
   a cluster [24][25][45][53][73]. warte targets 2-vCPU VPSes and a Raspberry Pi.
5. **Averages for ETA.** Use percentiles; an average queue time looks fine while
   the first build of every morning takes 20 minutes [15][30].
6. **Long-lived agent secrets.** Jenkins' JNLP secret model means a leaked
   secret is valid until rotated [8]. Prefer short-lived certs (warte, SPIRE).
7. **Assuming the vendor's mobile app covers self-hosted.** Grafana's official
   app does not support Grafana OSS [80]. Build warte's own clients (F7).
8. **Mobile mTLS as an afterthought.** It is painful: p12 import, full-chain
   sending bugs, years of feature requests (Bitwarden landed it in 2026.4.1)
   [140][141]. Design device-bound certs + biometrics in from day one.
9. **Silent multi-master merges.** Without divergence detection, two instances
   will quietly disagree and nobody will know. warte's version-vector distance
   alerting is the fix — keep it.
10. **Copyleft dependencies in a distributable product.** AGPL (Grafana,
    Chronograf, Windmill, Daytona, Coder, FUXA) and GPL (Zabbix, Icinga, Netdata,
    Buildbot, Laminar) are fine for internal use but constrain distribution
    [79][82][47][76][77][103][85][86][88][21][23]. warte is MIT — keep its
    dependency tree permissive.
11. **Building a portal instead of a tool.** The IDP market's core failure mode
    is buying/building a catalog nobody uses [64][67]. warte's value is
    operational (ask, control, revoke), not informational.

---

## 12. Sources

### warte (primary)
1. warte README — `README.md` (repo) / https://github.com/pulseengine/warte
2. warte architecture — `docs/architecture.md` (repo)
3. warte issue #4 — "Feature: ask (ETA estimation from measured waits)" — https://github.com/pulseengine/warte/issues/4
4. PulseEngine blog — "The orchestrator: who does what" — https://pulseengine.eu/blog/the-orchestrator-who-does-what/

### Category A — CI dashboards / CI servers
5. Jenkins — https://www.jenkins.io/ ; repo (MIT, 26.7k★) https://github.com/jenkinsci/jenkins
6. Jenkins Build Monitor Plugin — https://plugins.jenkins.io/build-monitor-plugin/
7. Jenkins Dashboard View plugin — https://plugins.jenkins.io/dashboard-view/
8. Jenkins security/services + inbound agents — https://www.jenkins.io/doc/book/security/services ; https://github.com/jenkinsci/remoting/blob/master/docs/inbound-agent.md
9. GitLab CI/CD analytics — https://docs.gitlab.com/user/analytics/ci_cd_analytics/
10. GitLab Runner fleet scaling — https://docs.gitlab.com/runner/fleet_scaling/
11. GitLab CI pipelines Grafana dashboard — https://grafana.com/grafana/dashboards/10620-gitlab-ci-pipelines/
12. gitlab-ci-dashboard (MIT) — https://github.com/ci-dashboard/gitlab-ci-dashboard
13. ismoilovdevml/gitlab-ci-dashboard (MIT) — https://github.com/ismoilovdevml/gitlab-ci-dashboard
14. Buildkite — https://buildkite.com/ ; agent repo (MIT) https://github.com/buildkite/agent
15. Buildkite queue metrics — https://buildkite.com/docs/pipelines/insights/queue-metrics
16. Buildkite queues — https://buildkite.com/docs/agent/queues
17. Concourse — https://concourse-ci.org/ (Apache-2.0)
18. Concourse dashboard docs — https://concourse-ci.org/docs/observation/
19. Woodpecker CI — https://woodpecker-ci.org/about ; repo (Apache-2.0, 8.0k★) https://github.com/woodpecker-ci/woodpecker
20. GoCD — https://www.gocd.org/ ; repo (Apache-2.0, 7.4k★) https://github.com/gocd/gocd
21. Buildbot — https://buildbot.net/ ; repo (GPL-2.0, 5.5k★) https://github.com/buildbot/buildbot
22. Strider — https://github.com/Strider-CD/strider (BSD; last push 2024)
23. Laminar — https://laminar.ohwg.net/ ; repo (GPL-3.0) https://github.com/ohwgiles/laminar
24. Tekton — https://tekton.dev/ (Apache-2.0, CNCF incubating)
25. Prow — https://github.com/kubernetes-sigs/prow (Apache-2.0)
26. Zuul — https://zuul-ci.org/ (Apache-2.0)
27. Dagger — https://dagger.io/ ; repo (Apache-2.0, 16.5k★) https://github.com/dagger/dagger
28. Earthly — https://earthly.dev/blog/earthly-open-source/ (MPL-2.0; maintenance ended Jul 2025)
29. GitHub Actions metrics — https://docs.github.com/en/actions/concepts/metrics
30. WarpBuild — "GitHub Actions Queue Times and How to Fix Them" — https://www.warpbuild.com/guides/github-actions-queue-times
31. BuildPulse — "Self-hosted vs GitHub-hosted runners: queue time" — https://buildpulse.io/blog/self-hosted-vs-github-hosted-runners-queue-time
32. gh-dash — https://github.com/dlvhdr/gh-dash (MIT, 12.7k★)
33. Snorlx (self-hosted GitHub Actions dashboard) — https://blog.devops.dev/a-self-hosted-ci-cd-dashboard-for-github-actions-60e382ab4468
34. github-dash — https://github-dash.com/
35. gitdeck — https://github.com/debba/gh-dashboard
36. Travis CI apps/clients list (CCMenu, CatLight, BuildNotify) — https://docs.travis-ci.com/user/apps/ ; https://github.com/anaynayak/buildnotify
37. Selfhostindex — Jenkins alternatives — https://selfhostindex.com/alternatives/jenkins-proprietary/
38. awesome-cicd — https://github.com/brandonhimpfen/awesome-cicd
39. awesome-ci — https://github.com/ligurio/awesome-ci

### Category B — Agent orchestration
40. Temporal — https://temporal.io/ ; repo (MIT) https://github.com/temporalio/temporal
41. (see 40) Temporal evolved from Cadence — https://www.zenml.io/blog/temporal-vs-airflow
42. Apache Airflow — https://airflow.apache.org/ (Apache-2.0)
43. Prefect — https://github.com/PrefectHQ/prefect (Apache-2.0, 24.1k★)
44. Dagster — https://github.com/dagster-io/dagster (Apache-2.0)
45. Argo Workflows — https://argoproj.github.io/workflows/ (Apache-2.0, CNCF graduated)
46. Kestra — https://kestra.io/ (Apache-2.0)
47. Windmill — https://github.com/windmill-labs/windmill (AGPL-3.0)
48. Flyte — https://flyte.org/ (Apache-2.0)
49. Netflix Conductor — https://conductor-oss.org/ (Apache-2.0)
50. Cadence (Uber) — https://github.com/uber/cadence (MIT, archived)
51. HashiCorp Nomad — https://www.nomadproject.io/ (BSL-1.1 since 1.7) ; license note https://cloudparts.dev/technologies/nomad/
52. HashiCorp Consul — https://developer.hashicorp.com/consul (BSL-1.1)
53. Actions Runner Controller — https://github.com/actions/actions-runner-controller (Apache-2.0, 6.6k★) ; docs https://docs.github.com/en/actions/concepts/runners/actions-runner-controller
54. GitHub Actions runner — https://github.com/actions/runner (MIT)
55. Mage — https://github.com/mage-ai/mage-ai (Apache-2.0)
56. Rundeck — https://www.rundeck.com/ (Apache-2.0 core)
57. StackStorm — https://stackstorm.com/ (Apache-2.0)
58. AWX — https://github.com/ansible/awx (Apache-2.0)
59. SaltStack — https://github.com/saltstack/salt (Apache-2.0)
60. SPIFFE — https://spiffe.io/ ; SPIRE repo (Apache-2.0, CNCF) https://github.com/spiffe/spire
61. SPIRE concepts / registering workloads — https://spiffe.io/docs/latest/spire-about/spire-concepts/ ; https://spiffe.io/docs/latest/deploying/registering/
62. awesome-workflow-engines — http://meirwah.github.io/awesome-workflow-engines/
63. Airflow vs Prefect vs Dagster (2026) — https://dev.to/keshav07/airflow-vs-dagster-vs-prefect-how-to-choose-211o

### Category C — IDPs / developer control towers
64. Backstage — https://backstage.io/ ; repo (Apache-2.0) https://github.com/backstage/backstage
65. Backstage CNCF project page (incubating since 2022-03-15) — https://www.cncf.io/projects/backstage/
66. Backstage staffing/TCO — https://roadie.io/blog/7-best-developer-portals-for-enterprise-engineering-teams/ ; https://kubernetesguru.com/blog/backstage-vs-cortex/
67. Backstage vs Port vs Cortex TCO — https://squareops.com/blog/backstage-vs-port-vs-cortex/ ; https://wetheflywheel.com/en/comparisons/backstage-vs-port-vs-cortex/
68. Port — https://www.port.io/ (proprietary SaaS)
69. Cortex — https://www.cortex.io/ (proprietary; $65/user/mo list)
70. OpsLevel — https://www.opslevel.com/ (proprietary; $39–41/user/mo list)
71. Roadie — https://roadie.io/ (proprietary SaaS, active)
72. Humanitec — https://humanitec.com/ (proprietary; self-host option)
73. Kratix — https://kratix.io/ (Apache-2.0)
74. Mia-Platform — https://mia-platform.eu/
75. DevPod — https://devpod.sh/ (MPL-2.0)
76. Daytona — https://www.daytona.io/ (AGPL-3.0)
77. Coder — https://coder.com/ (AGPL-3.0)
78. IDP pricing comparison — https://developerportalcost.com/platforms ; landscape https://internaldeveloperplatform.org/ ; https://encore.dev/articles/platform-engineering-tools

### Category D — Factory / operations dashboards
79. Grafana — https://grafana.com/ ; repo (AGPL-3.0, 77.5k★) https://github.com/grafana/grafana
80. Grafana mobile app (Play Store: "The app does not support Grafana OSS") — https://play.google.com/store/apps/details?id=com.grafana.oncall.prod ; docs https://grafana.com/docs/grafana-cloud/platform/mobile-app/
81. Kibana — https://www.elastic.co/kibana (Elastic License 2.0)
82. Chronograf — https://github.com/influxdata/chronograf (AGPL-3.0)
83. Prometheus — https://prometheus.io/ (Apache-2.0, CNCF graduated)
84. Alertmanager — https://github.com/prometheus/alertmanager (Apache-2.0)
85. Zabbix — https://www.zabbix.com/ (GPL-2.0)
86. Icinga 2 — https://icinga.com/ (GPL-2.0)
87. Checkmk — https://checkmk.com/ (GPL-2.0 community)
88. Netdata — https://www.netdata.cloud/ (GPL-3.0)
89. Nagios Core — https://www.nagios.org/ (GPL-2.0)
90. LibreNMS — https://www.librenms.org/ (GPL-3.0)
91. Uptime Kuma — https://github.com/louislam/uptime-kuma (MIT)
92. Gatus — https://github.com/TwiN/gatus (Apache-2.0)
93. Cachet — https://cachethq.io/ (BSD-3-Clause)
94. Vigil — https://github.com/valeriansaliou/vigil (MPL-2.0)
95. Home Assistant — https://www.home-assistant.io/ (Apache-2.0)
96. Home Assistant Android app — https://github.com/home-assistant/android
97. ThingsBoard — https://thingsboard.io/ ; license change https://thingsboard.io/blog/one-thingsboard-source-available/ (Apache-2.0 CE < 4.4; BSL-1.1 from 4.4)
98. ThingsBoard Community Grant License Agreement — https://thingsboard.io/legal/community-grant-license-agreement/
99. United Manufacturing Hub — https://www.umh.app/ (Apache-2.0)
100. UMH + Grafana — https://grafana.com/blog/industrial-iot-visualization-why-united-manufacturing-hub-chose-grafana-to-power-its-iiot-platform/
101. Apache StreamPipes — https://streampipes.apache.org/ (Apache-2.0, TLP)
102. Node-RED — https://nodered.org/ (Apache-2.0)
103. FUXA — https://github.com/frangoteam/FUXA (AGPL-3.0)
104. RapidSCADA — https://rapidscada.org/
105. OpenSCADA — https://oscada.org/
106. ScadaBR — https://github.com/ScadaBR/ScadaBR
107. Tulip — https://tulip.co/ (proprietary)
108. Litmus — https://litmus.io/ (commercial)

### Category E — cross-cutting infrastructure
109. HPC job queue wait-time prediction (PEARC 2025) — https://doi.org/10.1145/3708035.3736067
110. Google SubmitQueue build-time prediction — https://arxiv.org/pdf/2501.03440
111. Empirical study of long CI builds (ACM EMSE) — https://dl.acm.org/doi/10.1007/s10664-019-09695-9
112. TransitApp ETA-Accuracy-Benchmark — https://github.com/TransitApp/ETA-Accuracy-Benchmark
113. step-ca — https://smallstep.com/docs/step-ca/ ; repo (Apache-2.0) https://github.com/smallstep/certificates
114. cfssl — https://github.com/cloudflare/cfssl (BSD-2-Clause)
115. EJBCA — https://www.ejbca.org/ (LGPL)
116. HashiCorp Vault — https://developer.hashicorp.com/vault (BSL-1.1)
117. Casbin — https://casbin.org/ (Apache-2.0)
118. OpenFGA — https://openfga.dev/ (Apache-2.0)
119. SpiceDB — https://authzed.com/spicedb (Apache-2.0)
120. OPA — https://www.openpolicyagent.org/ (Apache-2.0)
121. Cedar — https://cedarpolicy.com/ (Apache-2.0)
122. Cerbos — https://www.cerbos.dev/ (Apache-2.0)
123. Authorization tools comparisons — https://authzed.com/blog/casbin ; https://startwithidentity.com/articles/top-7-open-source-authorization-tools/ ; https://authzed.com/learn/openfga-alternatives
124. GitHub community — "How is the Avg queue time in GitHub Actions Insights calculated?" — https://github.com/orgs/community/discussions/188437
125. Harbor — https://goharbor.io/ (Apache-2.0, CNCF graduated)
126. Harbor CNCF project page — https://www.cncf.io/projects/harbor/
127. Nexus Repository OSS — https://github.com/sonatype/nexus-public (EPL-1.0)
128. JFrog Artifactory — https://jfrog.com/artifactory/
129. ORAS — https://oras.land/ (Apache-2.0)
130. Sigstore — https://docs.sigstore.dev/about/overview/ (Apache-2.0)
131. in-toto — https://in-toto.io/ (Apache-2.0)
132. Automerge — https://github.com/automerge/automerge (MIT)
133. Yjs — https://github.com/yjs/yjs (MIT)
134. AntidoteDB — https://github.com/AntidoteDB/antidote (Apache-2.0)
135. Eclipse Ditto — https://github.com/eclipse/ditto (EPL-2.0)
136. Litestream — https://litestream.io/ (Apache-2.0)
137. rqlite — https://rqlite.io/ (MIT)
138. CRDT implementations — https://crdt.tech/implementations
139. awesome-crdt — https://github.com/alangibson/awesome-crdt
140. Bitwarden mTLS on mobile — https://community.bitwarden.com/t/mtls-support-on-mobile/97180
141. immich mTLS discussion — https://github.com/immich-app/immich/discussions/1611
142. Kibana vs Grafana — https://signoz.io/blog/kibana-vs-grafana/
143. Monitoring tools comparisons — https://checkmk.com/blog/best-zabbix-alternatives ; https://computingforgeeks.com/10-best-open-source-linux-monitoring-tools/
144. Open-source IoT dashboards — https://nodrix.live/guides/open-source-iot-dashboard
145. SCADA comparison — https://mdcplus.fi/blog/top-free-scada-manufacturing-automation/
146. Backstage alternatives — https://www.port.io/blog/top-backstage-alternatives
147. Self-hosted runners comparison — https://devopsboys.com/blog/self-hosted-runners-vs-gitlab-vs-buildkite-2026
148. JetBrains — Best CI/CD Tools 2026 — https://blog.jetbrains.com/teamcity/2026/03/best-ci-tools/
149. AWS — self-hosted GitHub runners at scale — https://aws.amazon.com/blogs/devops/best-practices-working-with-self-hosted-github-action-runners-at-scale-on-aws/
150. RunsOn benchmarks methodology — https://runs-on.com/benchmarks/methodology/
151. Kestra vs Windmill — https://openalternative.co/compare/kestra/vs/windmill
152. PKI comparison (EJBCA vs Smallstep vs Vault) — https://www.qcecuring.com/blog/ejbca-vs-smallstep-vs-vault-pki
153. State of OSS workflow orchestration 2025 — https://www.pracdata.io/p/state-of-workflow-orchestration-ecosystem-2025
154. awesome-status-pages — https://github.com/ivbeg/awesome-status-pages
155. GitLab licensing (core MIT) — https://docs.gitlab.com/development/licensing/

---

*Report ends. Research only — no code was written or changed.*

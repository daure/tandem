# Development fixtures

Generate six source repositories, seven instance templates, and four event providers under the ignored `projects/` directory.
Python 3.11+, Git, local Docker Engine, and Docker Compose 2.20+ are required.

| Template | Repositories | Minimal feature | Coverage |
|---|---|---|---|
| `guestbook` | `guestbook` | Read and edit a greeting in memory | Monorepo, frontend/backend build contexts |
| `greetings` | `greetings-api`, `greetings-ui` | Save a greeting in PostgreSQL | Multiple repos, API-owned Compose include, migration, persistent volume |
| `postcard` | `postcard` | Static hello-world page | Assets, nested links, redirects, fast baseline |
| `mailroom` | `mailroom` | Queue a greeting and poll its delivery | Redis, worker heartbeat, asynchronous completion |
| `repo-only` | `repo-only` cloned as `app` | Local Python greeting and unit tests | Manifest-only template, Git Setup row, checkout-path copy, local guidance |
| `compose-only` | — | Redis with a healthcheck and persistent volume | Compose without a manifest, service-only guidance, no routes |
| `guidance-only` | — | Scratch workspace for notes and research | Optional manifest, custom `AGENTS.md`, nested `tandem-files/` seeds, no Git or Docker |

## Generate and run

Run from the Tandem checkout:

```sh
python3 projects-generators/generate.py --seed-commits
cargo run -- dev
```

To append the three minimal fixtures to an existing `projects/` setup:

```sh
python3 projects-generators/generate.py --add-minimal --seed-commits
```

This refuses occupied minimal-fixture paths, preserves existing sources/templates/workspaces and
environment settings, and adds the fixture names to the inventory. Refresh templates in the TUI afterward.
If `repo-only` and `compose-only` are already installed, use `--add-guidance` to add only `guidance-only`;
that operation creates no repository and needs no seed commit.

For a focused live lifecycle check, build Tandem and run
`python3 projects-generators/tests/live_smoke.py --minimal`; it uses an isolated temporary fixture root
and cleans up its containers, networks and volumes.

The TUI lists all seven templates. Start one with a distinct instance name, such as `guestbook-one`.
Web fixtures use `http://localhost:9886/guestbook-one/web/`; APIs use the sibling `api/` route.
`repo-only` completes without Docker and exposes its checkout beneath Setup; `yy` copies its absolute
directory. Run `python3 -m unittest discover -s tests -v` there. `compose-only` has no HTTP URL;
use its generated Compose command with `exec -T redis redis-cli ping` and expect `PONG`.
`guidance-only` creates `AGENTS.md` with scratch-workspace instructions and copies `tandem-files/`
contents into the workspace: `notes.md`, `research/sources.md`, and `experiments/README.md`.
Open its template details and select **Files** to browse the seed tree. Edit, add, or remove a seed
file to see the tree refresh; removing all seed files or the folder hides that tab.
Start a fresh instance to copy the current seeds, then follow its guidance to record findings locally.
Existing workspace files remain user-owned when template seeds change.
Expand `greetings` Setup to compare its sorted Git rows with the migration job beneath them.
On Unix, every `cargo run` invocation of Tandem sources the checkout's `projects/env.sh` when it
exists, including plain TUI, `dev`, `new-instance` and MCP modes. Its exports override inherited
values in the child process; the calling shell stays unchanged. Treat this file as trusted shell
code. A missing file leaves the inherited environment intact. Cargo test binaries and release
helpers retain their inherited environment. To use another fixture root, source that root's
`env.sh` and invoke the built binary directly, for example `target/debug/tandem dev`.
The generated environment isolates Tandem storage, Docker resource names, and the gateway port;
Tandem's development HTTP MCP listener still uses its configured/default address.

`--seed-commits` explicitly authorizes one seed commit per source repository and one for the shared template catalog. Without it, repositories
have empty histories and must be committed before cloning. The generator never commits the Tandem
checkout or updates Git configuration. Seed commits use a fixed fixture identity and timestamp;
normal Git hooks and signing settings still apply.

`--output PATH` selects another fixture root. `--gateway-port PORT` defaults to 9886; use a distinct
free port for each concurrently running fixture root. The namespace is derived from the absolute
output path. Runtime generation needs no GitHub account or external Git remote; first startup pulls
container images and Python packages. Package versions and image release tags are specified, but
image tags are not content-addressed locks.

## Event providers

The developer catalog includes synthetic `slack`, `jira`, `datadog`, and `github` packages.
They require no external accounts or API connections. Slack uses the `message` profile for
`messages` and `reactions`; Jira uses `ticket` for `backlog`; Datadog uses `system_event` for
`production-gateway-issue`; GitHub uses `generic` for `releases`.
Each owns a Dockerfile, `src/provider.py`, normalized `sample.json`, and a validated `provider.json`.
The manifest declares `schema_version: 1`, a unique `name`, supported `profile`, `description`,
`protocol: "tandem-events-v1"`, `streams`, `stream_control: true`, and optional `feedback` names. Tandem manages one installation per
package and namespace. `sample.json` contains an ordered list of normalized event envelopes;
the runtime also accepts a single envelope. Edit the samples, then Stop and Start to rebuild.
Slack cycles through PR requests, support queries, and reactions to those messages.
Each emitted event has a one-based `metadata.stream_sequence` counting only its own stream,
and a zero-based `metadata.sample_sequence` counting all events from that provider.
Checkpoint state preserves the sequence across retries and restarts.
Python runs inside the sample containers;
the HTTP protocol is language-independent and Tandem requires no Python runtime.
The runtime polls authenticated stream controls, halts generation for each stopped stream before
acknowledgment, and clears its buffered events. Sibling streams continue; resume generates new events only.
Source catch-up is outside the fixture contract.

For an existing fixture root, append the packages without moving instance templates or changing
source repositories, workspaces, or Git history:

```sh
python3 projects-generators/generate.py --add-providers
```

After generating or appending providers, open Tandem from the checkout:

```sh
cargo run -- dev
```

Open **Providers**, select each package, and choose **Start provider**. Confirm trusted Docker
execution; Tandem provisions credentials, starts its own sidecar, builds the image, and runs the
collector. Use the same tab to Stop, inspect details, or read Logs. No manual Compose,
credential setup, or sidecar command is required.
Expand a provider to inspect its stream children. Stream menus and details act on that child;
provider Start enables all its streams and provider Stop halts the collector.

Open the second tab, **Events**. Four row styles arrive every three seconds, with source-specific
data, shared people/attachments/relations, bounded supporting context, and custom metadata.
Enter opens details and its per-rule Acceptances tab. Enabled predicates accept matching events
automatically; `r` requests confirmed replay of any retained event using current enabled rules.
`p` selects the originating provider; Providers' **Events** action shows only that source.
The provider multiselect filters sources; an empty selection shows all providers.

### Developer rules

With the `guidance-only` fixture installed, add six disabled example rules:

```sh
cargo run -- rules-setup --model openai/gpt-6.1-sol-fast
```

Setup preserves existing definitions and enablement. Use `rules-setup --model openai/gpt-6.1-sol-fast --refresh`
to apply the example predicates and descriptions to saved example rules. Changed rules are paused;
their model, template, prompt, service-start setting, and destination remain intact.
**Rules** follows Instances; `[` and `]`
visit every main tab in both directions. Enter opens a rule's dialog. Review **Settings**, including
its independent handler template, model, and initial prompt; valid edits save automatically.
Editing pauses an enabled rule. Space toggles the rule
and asks for authorization before enabling automatic actions.

| Rule | Provider / stream | Output |
|---|---|---|
| `slack-pr-request` | `slack` / `messages` | `pr-review-brief.md` |
| `slack-support-query` | `slack` / `messages` | `support-checklist.md` |
| `slack-message-reaction` | `slack` / `reactions` | `reaction-summary.md` |
| `jira-ticket-triage` | `jira` / `backlog` | `ticket-triage.md` |
| `datadog-gateway-issue` | `datadog` / `production-gateway-issue` | `gateway-incident-brief.md` |
| `github-release-notes` | `github` / `releases` | `release-summary.md` |

Each rule matches its provider and stream, requires `metadata.fixture == true`, and checks
`metadata.stream_sequence % 3 == 0`: stream events 3, 6, 9, and so on. Matching is independent
of message content, channel, severity, and status. Both Slack message rules accept the same third
message, demonstrating independent acceptances. Their tasks report when the relevant request is absent.
Each match creates its own fresh `guidance-only` scratch instance and OpenCode session.
Handlebars prompts show optional nested message fields with `if`, attachments with `each` and an
`else` fallback, and the full event through `{{json event}}`.
At the default three-second interval, Slack rules match every eighteen seconds; other providers'
rules match every nine seconds. Enabling all six produces work faster than Tandem's ten-starts-per-minute
dispatch limit, so acceptances queue. Enable a subset or increase the provider interval for model runs.

After rebuilding Tandem, restart the development TUI and its owned event sidecar before enabling
examples. Existing sidecars retain the executable they started with.
No model prompts run while the examples remain disabled. Observe matched events in Events and
follow acceptance links to their instances and sessions; a launched session does not prove file creation.

Tandem retains provider-scoped credentials in SQLite and atomically writes private token files
under `$TANDEM_HOME/provider-credentials/$TANDEM_NAMESPACE/`. Containers mount their own token file
read-only; credential values stay out of template files and Compose environment declarations.
Named volumes hold pending batches, checkpoint identities, and deduplicated notification records.

The managed collectors use Linux Docker host networking to reach Tandem's detached loopback sidecar,
independently of MCP. Tandem supplies its verified address. Collection persists after the TUI closes;
the TUI supervises the sidecar when reopened, and Start repairs an interrupted sidecar.
A disconnected provider retains its pending batch and
retries it with identical IDs. It polls, persists, and acknowledges `received`, `replayed`, and
`acknowledged` feedback. Read its output through **Logs** in Providers.

Stop preserves checkpoints and event history. Stop immediately discards incoming batches
before Docker work. The sample runtime advances its checkpoint on explicit discarded IDs so those
events are not retried after activation. Start enables ingestion; failed Stop
operations keep it disabled. The sample runtime handles SIGTERM by interrupting pending work and
closing its checkpoint database; pending batches remain available for retry. Package deletion does not remove an installed collector's
ownership evidence, so Tandem can still Stop it. The generated `compose.providers.yaml` and
`providers-setup` command support isolated protocol tests; normal lifecycle uses Tandem's saved
private launch configuration. The protocol-test harness can override sample batch size and interval.

Run isolated verification after building:

```sh
python3 projects-generators/tests/events_smoke.py
python3 projects-generators/tests/events_smoke.py --docker
python3 projects-generators/tests/providers_smoke.py
```

The event checks use temporary homes and a free loopback port. They verify all four profiles,
five streams, seventy-two distinct events, provider feedback acknowledgment, duplicate ingestion,
and sidecar restart persistence. MCP previews verify all six disabled rules match stream events
3, 6, and 9, including prompt rendering. No model prompts run.
The Docker variant builds all four images and removes its own containers and volumes.
The lifecycle check starts all four providers through Tandem, proving automatic setup,
stop/start checkpoint preservation, bounded logs, sidecar recovery, and Stop after package deletion.
The lifecycle check requires clean collector exits and reports measured Stop durations.
These checks clean up their isolated containers, volumes, and sidecar workers; they do not touch the
active developer fixture environment or OpenCode clients.

## Where things live

```text
projects/
  guestbook/                 # .git, backend/, frontend/, compose.yaml
  greetings-api/             # .git, API, migration, infra/compose.yaml
  greetings-ui/              # .git, browser UI
  postcard/                  # .git, static site
  mailroom/                  # .git, backend/, frontend/, Redis infrastructure
  repo-only/                 # .git, greeting.py, tests/, AGENTS.md; local execution
  env.sh                    # source this to select the fixture environment
  fixtures.json             # generated inventory and environment
  .tandem/
    templates/              # one Git repository for both template catalogs
      instances/<template>/ # Compose and/or manifest recipes with tandem-agents.md
      providers/<source>/   # slack, jira, datadog, github: Dockerfile, src/, sample.json, provider.json
    workspaces/<instance>/  # independent writable clones of the relevant repos
```

`projects-generators/` is the tracked source of fixture recipes and app assets. Edit it to change
what fresh fixtures contain. Each generated source repository is also editable and independently
versioned; those edits belong to that local fixture set.

The scratch-workspace seeds live in `projects-generators/assets/guidance-only/`; the generator places
them under `.tandem/templates/instances/guidance-only/tandem-files/`. Existing generated templates are editable:
add the same seed paths there to try the example without regenerating or overwriting a fixture root.

Each template's `tandem-agents.md` describes its services, workspace mounts, edit/restart
behavior and a fixture-specific evaluation cycle. The tracked originals live in
`projects-generators/assets/guidance/`. They appear in the template's Guidance tab and
are appended to newly generated workspace `AGENTS.md` files. Browser guidance uses
host-installed `agent-browser` and its version-matched `skills get core --full` guide.

Repository-bearing templates declare repositories in `tandem.json`. Tandem clones them with host Git before any Compose,
under your UID/GID, using `--no-local` so objects do not depend on shared hardlinks. With Branch
instances enabled, it checks out the instance-named branch when available or creates it locally from
the source's default branch. With the setting disabled, it checks out the default branch.
New branches remain in the workspace until explicitly pushed. The clone's `origin` is the host's
absolute source-repository path. Existing clones retain their branch, commits, staged changes and
untracked files. Unexpected origins and occupied non-repository paths cause a visible failure.
Existing generated templates are editable resources; migrate them by declaring sources and targets
and removing clone services/dependencies, or generate a separate fixture root.

New source commits are available to instance clones through `git fetch origin`; updating an existing
clone is an explicit Git operation. A normal source repository rejects pushes to its checked-out
branch by default; push a feature branch if needed. Uncommitted source edits are not cloned.

Images build from source-repository Dockerfiles; application processes read code from workspace
clones. Edit workspace assets and refresh the browser. Restart the affected container after Python
code changes. To change dependencies or image build rules, edit the source repository and start the
instance from its template again; startup builds images using Docker's layer cache. Template
infrastructure is shared across instances; database volumes,
Redis volumes, and source workspaces are instance-specific.

The web fixture source repositories also have standalone Compose infrastructure. From that repository,
`docker compose up --build -d` starts it; its README lists the loopback URL. Greetings API owns its
database and migration. Start the full Greetings app from `greetings-ui`: it includes its sibling
API's Compose file and proxies to the internal API service. Start from `greetings-api` to run only
the API and database. Set `WEB_PORT` and `API_PORT` to override standalone ports. Under Tandem, only
the shared gateway publishes ports, and browser requests use prefix-aware sibling service URLs.

## Failures and lifecycle

Copy a template's `.env.example` to `.env` and change the relevant value before starting a fresh instance:

| Setting | Templates | Expected result |
|---|---|---|
| `FAIL_READINESS=1` | Guestbook, Greetings, Mailroom | API health returns 503 |
| `STARTUP_DELAY=10` | Guestbook, Greetings, Mailroom | API readiness is delayed |
| `FAIL_MIGRATION=1` | Greetings | Migration exits; API cannot start |
| `FAIL_WORKER=1` | Mailroom | Worker exits; API cannot become ready |

Fault settings are template-wide, so use fresh instance names and avoid modifying a recipe another
developer is using. Inspect operation output and container logs; clear the setting before retrying.
To exercise repository failure, point a declared source at a missing path for a fresh instance;
startup fails before Compose and succeeds after restoring the source and retrying.
Mailroom is a minimal queue demo: it does not recover a job lost if a worker dies after dequeue.

Stopping through Tandem preserves workspaces and named data volumes. Generation refuses any occupied
fixture repository, runtime root, or generated metadata path; it never resets existing work. For a
fresh set, choose another output directory. Before manually deleting a fixture root, stop its instances
and gateway, and decide whether to retain its data volumes; deleting files does not remove Docker resources.
These apps use demo credentials, share a browser origin, and have no authentication. Keep them local.

## Verification

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -W error -m unittest discover -s projects-generators/tests -v
cargo build
PYTHONDONTWRITEBYTECODE=1 python3 -W error projects-generators/tests/live_smoke.py
```

The live smoke test creates its own temporary seeded fixture root and Docker namespace, exercises the
real Tandem MCP boundary, and removes only its own runtime resources and volumes. It does not mutate
the development fixtures. Run it with local Docker access; allow time for first-use image builds.
Use `--image-reuse` for a focused check that reuses one instance name across Guestbook, Greetings,
Mailroom, and Greetings while retaining Compose image tags between deletions.
Use `--concurrent` to verify twelve simultaneous Greetings startups, durable completion records,
and healthy gateway routes; it disables OpenCode history cleanup in its isolated fixture root.

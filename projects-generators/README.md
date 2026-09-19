# Development fixtures

Generate six Git repositories and seven Tandem templates under the ignored `projects/` directory.
Python 3.11+, Git, local Docker Engine, and Docker Compose 2.20+ are required.

| Template | Repositories | Minimal feature | Coverage |
|---|---|---|---|
| `guestbook` | `guestbook` | Read and edit a greeting in memory | Monorepo, frontend/backend build contexts |
| `greetings` | `greetings-api`, `greetings-ui` | Save a greeting in PostgreSQL | Multiple repos, API-owned Compose include, migration, persistent volume |
| `postcard` | `postcard` | Static hello-world page | Assets, nested links, redirects, fast baseline |
| `mailroom` | `mailroom` | Queue a greeting and poll its delivery | Redis, worker heartbeat, asynchronous completion |
| `repo-only` | `repo-only` cloned as `app` | Local Python greeting and unit tests | Manifest-only template, Git Setup row, checkout-path copy, local guidance |
| `compose-only` | — | Redis with a healthcheck and persistent volume | Compose without a manifest, service-only guidance, no routes |
| `guidance-only` | — | Scratch workspace for notes and research | Guidance-only discovery, empty workspace, custom `AGENTS.md`, no Git or Docker |

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
`guidance-only` creates a folder containing `AGENTS.md` with the template's scratch-workspace instructions;
open the workspace and follow that guidance to create notes locally.
Expand `greetings` Setup to compare its sorted Git rows with the migration job beneath them.
On Unix, every `cargo run` invocation of Tandem sources the checkout's `projects/env.sh` when it
exists, including plain TUI, `dev`, `new-instance` and MCP modes. Its exports override inherited
values in the child process; the calling shell stays unchanged. Treat this file as trusted shell
code. A missing file leaves the inherited environment intact. Cargo test binaries and release
helpers retain their inherited environment. To use another fixture root, source that root's
`env.sh` and invoke the built binary directly, for example `target/debug/tandem dev`.
The generated environment isolates Tandem storage, Docker resource names, and the gateway port;
Tandem's development HTTP MCP listener still uses its configured/default address.

`--seed-commits` explicitly authorizes one seed commit per fixture repository. Without it, repositories
have empty histories and must be committed before cloning. The generator never commits the Tandem
checkout or updates Git configuration. Seed commits use a fixed fixture identity and timestamp;
normal Git hooks and signing settings still apply.

`--output PATH` selects another fixture root. `--gateway-port PORT` defaults to 9886; use a distinct
free port for each concurrently running fixture root. The namespace is derived from the absolute
output path. Runtime generation needs no GitHub account or external Git remote; first startup pulls
container images and Python packages. Package versions and image release tags are specified, but
image tags are not content-addressed locks.

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
    templates/              # Compose and/or manifest recipes with tandem-agents.md
    workspaces/<instance>/  # independent writable clones of the relevant repos
```

`projects-generators/` is the tracked source of fixture recipes and app assets. Edit it to change
what fresh fixtures contain. Each generated source repository is also editable and independently
versioned; those edits belong to that local fixture set.

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
code changes. To change dependencies or image build rules, edit the source repository and explicitly
rebuild the template's image. Template infrastructure is shared across instances; database volumes,
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

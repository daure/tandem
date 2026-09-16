# Development fixtures

Generate five Git repositories and four Tandem templates under the ignored `projects/` directory.
Python 3.11+, Git, local Docker Engine, and Docker Compose 2.20+ are required.

| Template | Repositories | Minimal feature | Coverage |
|---|---|---|---|
| `guestbook` | `guestbook` | Read and edit a greeting in memory | Monorepo, frontend/backend build contexts |
| `greetings` | `greetings-api`, `greetings-ui` | Save a greeting in PostgreSQL | Multiple repos, API-owned Compose include, migration, persistent volume |
| `postcard` | `postcard` | Static hello-world page | Assets, nested links, redirects, fast baseline |
| `mailroom` | `mailroom` | Queue a greeting and poll its delivery | Redis, worker heartbeat, asynchronous completion |

## Generate and run

Run from the Tandem checkout:

```sh
python3 projects-generators/generate.py --seed-commits
cargo run -- dev
```

The TUI lists all four templates. Start one with a distinct instance name, such as `guestbook-one`.
Its website is `http://localhost:9886/guestbook-one/web/`; APIs use the sibling `api/` route.
On Unix, `cargo run -- dev` sources the checkout's `projects/env.sh` when it exists, then starts
the combined TUI/HTTP MCP entry point. Its exports override inherited values in the child process;
the calling shell stays unchanged. Treat this file as trusted shell code. A missing file leaves the
inherited environment intact. Source it manually before plain `cargo run` or `cargo run -- mcp`.
To use another fixture root with development mode, source that root's `env.sh` and invoke the built
binary directly: `target/debug/tandem dev`.
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
  env.sh                    # source this to select the fixture environment
  fixtures.json             # generated inventory and environment
  .tandem/
    templates/              # four Compose recipes and tandem.json manifests
    workspaces/<instance>/  # independent writable clones of the relevant repos
```

`projects-generators/` is the tracked source of fixture recipes and app assets. Edit it to change
what fresh fixtures contain. Each generated source repository is also editable and independently
versioned; those edits belong to that local fixture set.

Templates run `repo-sync` as a one-shot service under your UID/GID before starting application
processes. It clones committed `main` using `--no-local` so objects do not depend on shared hardlinks.
The clone's `origin` is the host's absolute source-repository path, mounted read-only at the same path
inside the cloning container. Existing clones retain their branch, commits, staged changes, and
untracked files. Unexpected origins and occupied non-repository paths cause a visible failure.

New source commits are available to instance clones through `git fetch origin`; updating an existing
clone is an explicit Git operation. A normal source repository rejects pushes to its checked-out
branch by default; push a feature branch if needed. Uncommitted source edits are not cloned.

Images build from source-repository Dockerfiles; application processes read code from workspace
clones. Edit workspace assets and refresh the browser. Restart the affected container after Python
code changes. To change dependencies or image build rules, edit the source repository and explicitly
rebuild the template's image. Template infrastructure is shared across instances; database volumes,
Redis volumes, and source workspaces are instance-specific.

Each source repository also has standalone Compose infrastructure. From that repository,
`docker compose up --build -d` starts it; its README lists the loopback URL. Greetings API owns its
database and migration. Start the full Greetings app from `greetings-ui`: it includes its sibling
API's Compose file and proxies to the internal API service. Start from `greetings-api` to run only
the API and database. Set `WEB_PORT` and `API_PORT` to override standalone ports. Under Tandem, only
the shared gateway publishes ports, and browser requests use prefix-aware sibling service URLs.

## Failures and lifecycle

Copy a template's `.env.example` to `.env` and change the relevant value before starting a fresh instance:

| Setting | Templates | Expected result |
|---|---|---|
| `FAIL_CLONE=1` | All | Clone one-shot exits; apps cannot start |
| `FAIL_READINESS=1` | Guestbook, Greetings, Mailroom | API health returns 503 |
| `STARTUP_DELAY=10` | Guestbook, Greetings, Mailroom | API readiness is delayed |
| `FAIL_MIGRATION=1` | Greetings | Migration exits; API cannot start |
| `FAIL_WORKER=1` | Mailroom | Worker exits; API cannot become ready |

Fault settings are template-wide, so use fresh instance names and avoid modifying a recipe another
developer is using. Inspect operation output and container logs; clear the setting before retrying.
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

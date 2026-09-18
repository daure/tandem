## Greetings development

`web` serves `greetings-ui/public`; `api` runs `greetings-api/server.py`.
Both mount this workspace at `/workspace`.

### Reload, rebuild and migrate

| Changed files | Apply the change |
|---|---|
| `greetings-ui/public/*` | Refresh the browser. |
| `greetings-ui/server.py` | Restart the `web` container. |
| `greetings-api/server.py`, `store.py` | Restart the `api` container. |
| `greetings-api/schema.sql`, `migrate.py` | Rerun `migrate`, check its exit status/logs, then restart `api` if needed. |
| API `requirements.txt` or `Dockerfile` | Rebuild `api` and `migrate`; recreate/rerun `migrate` before recreating `api`. |
| UI `requirements.txt` or `Dockerfile` | Rebuild and recreate `web`. |

Use `docker restart` with discovered container IDs for Python changes. The stopped
`migrate` container can be rerun with `docker start -a` after SQL/script edits;
restarting `api` alone does not rerun migrations. The seed SQL creates the table
if missing; changing its `CREATE TABLE IF NOT EXISTS` does not alter an existing
table, so schema changes need appropriate SQL migrations.

For rebuilds, recover the rendered Compose file and working directory from container
labels `com.docker.compose.project.config_files` and
`com.docker.compose.project.working_dir`, and reuse this instance's project name.
Build contexts point at the source repositories `greetings-api` and `greetings-ui`;
dependency/Dockerfile edits must be in those contexts before building.

### Data and evaluation

For browser checks, use host-installed `agent-browser` with the web URL listed
above. Run `agent-browser skills get core --full` for its version-matched usage guide.

`db` is PostgreSQL, reachable as `db:5432` inside the environment. The database is
`greetings` and its demo user is `fixture`. The one-shot `migrate` service applies
`greetings-api/schema.sql` before `api` starts; a completed migration container is
expected to be stopped. Migration failures appear in its logs.

API paths are relative to the API URL listed above: `GET health`, `GET greeting`,
and `PUT greeting` with `{"message":"Hello from a check"}`. Messages are trimmed
and must contain 1–200 characters after trimming.

An evaluation cycle can save a greeting through the browser or API, compare
`GET greeting` with `SELECT message FROM greetings WHERE id = 1;` in `db`, then
refresh the browser. PostgreSQL's `psql` is available in `db`; connect as `fixture`
to `greetings`. The named database volume preserves the greeting across restarts.

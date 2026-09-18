## Mailroom development

`web` serves `mailroom/frontend/public`; `api` and `worker` run Python code in
`mailroom/backend`. These containers mount this workspace at `/workspace`.

### Reload and rebuild

| Changed files | Apply the change |
|---|---|
| `mailroom/frontend/public/*` | Refresh the browser. |
| `mailroom/frontend/server.py` | Restart `web`. |
| `mailroom/backend/server.py` | Restart `api`. |
| `mailroom/backend/worker.py` | Restart `worker`. |
| `mailroom/backend/store.py` | Restart both `worker` and `api`; both import this module. |
| Backend `requirements.txt` or `Dockerfile` | Rebuild and recreate both `worker` and `api`. |
| Frontend `requirements.txt` or `Dockerfile` | Rebuild and recreate `web`. |

Use `docker restart` with discovered container IDs for Python changes. Let test
jobs finish before restarting `worker`: a job dequeued by a killed worker is lost.
After a worker restart, wait for a fresh `worker:alive` heartbeat and successful
API `health` before testing delivery.

For rebuilds, recover the rendered Compose file and working directory from container
labels `com.docker.compose.project.config_files` and
`com.docker.compose.project.working_dir`, and reuse this instance's project name.
Build contexts are the source repository's `mailroom/backend` and
`mailroom/frontend`; dependency/Dockerfile edits must be in those contexts before building.

### Queue and evaluation

Use host-installed `agent-browser` with the web URL listed above to submit a
greeting and observe delivery. Run `agent-browser skills get core --full` for
its version-matched usage guide.

`redis` stores jobs and the queue at `redis:6379`, database 0; `redis-cli` is
available in that container. The `pending` list contains queued IDs,
`job:<id>` contains each job's JSON, and `worker:alive` is a five-second heartbeat.
API health depends on Redis and a current worker heartbeat.

API paths are relative to the API URL listed above. `POST jobs` with
`{"message":"Hello from a check"}` returns 202 with a job ID; `GET jobs/<id>`
eventually reports `status: "done"` and `result: "Delivered: Hello from a check"`.
Messages are trimmed and must contain 1–200 characters after trimming.

An evaluation cycle can submit through the browser or API, poll the returned ID,
compare the result with Redis's `job:<id>`, and inspect the worker's delivery log.
The queue may drain before inspection. The Redis volume persists data across restarts.

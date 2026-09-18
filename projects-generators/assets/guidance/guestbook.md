## Guestbook development

`web` and `api` mount this workspace at `/workspace`.

### Reload and rebuild

| Changed files | Apply the change |
|---|---|
| `guestbook/frontend/public/*` | Refresh the browser; assets are read from the workspace. |
| `guestbook/frontend/server.py` | Restart the `web` container. |
| `guestbook/backend/server.py`, `store.py` | Restart the `api` container; this resets the in-memory greeting. |
| Backend/frontend `requirements.txt` or `Dockerfile` | Rebuild and recreate `api` or `web`, respectively. |

Use `docker restart` with the discovered container IDs for Python changes; there is
no automatic Python reload. For rebuilds, recover the rendered Compose file and
working directory from container labels `com.docker.compose.project.config_files`
and `com.docker.compose.project.working_dir`, and reuse this instance's project name.
Build contexts are the source repositories' `guestbook/backend` and
`guestbook/frontend`, so dependency/Dockerfile edits must be present there before
building; workspace edits alone do not update these image inputs.

### Evaluation

For browser checks, use host-installed `agent-browser` with the web URL listed
above. Run `agent-browser skills get core --full` for its version-matched usage guide.

API paths are relative to the API URL listed above: `GET health`, `GET greeting`,
and `PUT greeting` with a JSON body such as `{"message":"Hello from a check"}`.
Messages are trimmed and must contain 1–200 characters after trimming.

A useful evaluation cycle is to save a greeting through the browser or API, read
it back with `GET greeting`, and refresh the browser to compare the displayed value.
The greeting lives in API process memory; restarting `api` restores the default
from `guestbook/backend/config.json`. There is no database to inspect.

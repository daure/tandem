## Postcard development

`web` runs `postcard/server.py` and serves `postcard/public` from this workspace,
mounted at `/workspace`. This fixture is a static site with no API or database.

### Reload and rebuild

Changes to `postcard/public/index.html`, `style.css`, and `about/index.html` are
served directly from the workspace: refresh the browser without rebuilding.
For `postcard/server.py` changes, use `docker restart` on the discovered `web`
container; the Python server has no automatic reload.

Changes to `requirements.txt` or `Dockerfile` require rebuilding and recreating
`web`. Recover the rendered Compose file and working directory from container
labels `com.docker.compose.project.config_files` and
`com.docker.compose.project.working_dir`, and reuse this instance's project name.
The build context is the source `postcard` repository; dependency/Dockerfile edits
must be present there before building, even though runtime files come from the workspace.

### Evaluation

Use host-installed `agent-browser` with the web URL listed above to check the
rendered page, links and redirects. Run `agent-browser skills get core --full`
for its version-matched usage guide.

Paths are relative to the web URL listed above. `index.html` is the landing page,
`style.css` supplies styling, `about/` is the second page, and `redirect` returns
a 302 redirect to `about/`.

An evaluation cycle can load the landing page, check stylesheet loading, follow
both links, and inspect `web` logs for failed requests. Also request `about`
without its trailing slash: the redirect should retain the instance's gateway
prefix, keeping navigation within the same environment.

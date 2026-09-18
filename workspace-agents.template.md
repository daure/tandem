# Workspace: {{instance}}

Repositories:
{{repositories}}

If a repository has `AGENTS.md` or `agents.md`, read it before working there; follow any applicable nested guidance.
Run Git commands inside the relevant repository.

## Docker Compose

Project: {{project}}.
{{services}}

Run any commands/tests via Docker/Compose or call exposed URLs/ports.
These development services and their data are disposable; preserve repository work.
Listed container ports are internal; use the listed HTTP URLs from the host.
```sh
dc() { {{compose_command}} "$@"; }
dc ps --all --format json # current container names, IDs, status and ports
dc exec -T SERVICE COMMAND
dc logs --tail 100 SERVICE
```

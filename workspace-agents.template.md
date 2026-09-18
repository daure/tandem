# {{instance}}

This workspace contains application repositories and Docker Compose development services.

## Services

{{repositories}}

Code paths in containers can differ from their configured working directories.

{{repository_guidance}}Edit source files and run Git commands in the checked-out repositories.
Run tools and tests locally when their dependencies are available; use the service containers when commands need the environment’s runtime or dependencies.
{{http_guidance}}
This Docker environment supports a self-evaluation loop for code changes: exercise the application running in its containers, inspect logs and database state, then use the results to refine and recheck the changes.

## Docker

Compose project: {{project}}

Use `{{compose_project_command}}` with service names for `exec` and `logs`;
use `{{compose_project_command}} ps --all` when container discovery or status is needed.

To run a command in a running service, substitute its service name and code path from the table:

```sh
{{compose_project_command}} exec -T -w CODE_PATH SERVICE COMMAND
```

For replicated services, use `exec --index N` to select a replica.
Omit `-w CODE_PATH` for services without a code path.
Building or recreating services also requires the instance's rendered Compose configuration.{{http_section}}

# {{instance}}

{{workspace_description}}

{{#inventory}}## {{inventory_heading}}

{{repositories}}

{{/inventory}}{{#code_paths}}Code paths in containers can differ from their configured working directories.

{{/code_paths}}{{#repositories}}{{repository_guidance}}Edit source files and run Git commands in the checked-out repositories.
{{/repositories}}{{#services}}Run tools and tests locally when their dependencies are available; use the service containers when commands need the environment’s runtime or dependencies.
{{http_guidance}}
Verify the running services with their tools and healthchecks, inspect relevant logs and data, then refine and recheck changes.

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
Building or recreating services also requires the instance's rendered Compose configuration.{{/services}}{{http_section}}

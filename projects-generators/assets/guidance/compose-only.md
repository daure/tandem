## Redis checks

The `redis` service uses append-only persistence in the instance's `redis-data` volume.
Use the generated project-scoped Compose command with `exec -T redis redis-cli ping` and
expect `PONG`. Run `exec -T redis redis-cli SET fixture:hello Tandem`, then
`exec -T redis redis-cli GET fixture:hello` and expect `Tandem`; clean up with
`exec -T redis redis-cli DEL fixture:hello` and expect `1`.

Redis writes diagnostics to stdout; use the generated Compose command with `logs redis`.
Stop and restart preserve its data. Deleting the instance removes its private volume.

# Manual status checks

Acceptance expectations for statuses and resources—not a record of passing manual tests.

## Setup

Run `cargo run -- dev` from the checkout; it loads the existing `projects/` fixtures.
Use fresh `status-*` instance names and the TUI Actions menu (`.`); Refresh is `R`.
Allow up to 10s for focused automatic discovery; health failures can take 60–90s.

For fault settings below, edit `projects/.tandem/templates/<template>/.env`.
Create it from `.env.example` only if absent; save and restore existing values after each test.
Keep other fault flags at zero during each case.
Settings affect the whole template: coordinate with other users and create fresh instances.

## Checklist

1. [ ] **Healthy + completed setup.** Create `status-card` from **postcard** and
   `status-greetings` from **greetings**, with fault settings at zero.
   Expect green **Healthy**; Greetings has `web`, `api`, `db` and a green **Completed**
   `migrate` job, excluded from its **3/3 running** count.

2. [ ] **Slow startup + hidden metrics.** Set Guestbook `STARTUP_DELAY=30`; create
   `status-slow`. Expect the native tuicore spinner during **Creating/Starting**,
   API **Checking health**, and waiting/not-started dependants.
   Instance/child metrics stay `—` until startup ends; then memory appears before CPU.

3. [ ] **Degraded + scoped restart.** Stop only `status-greetings/web` through Tandem.
   Expect gray **Stopped** on web and amber **Degraded · 2/3 running** on its instance.
   Restart web: blue activity spinner, then **Healthy**; other services keep their metrics.
   Web CPU warms up from its new run rather than reusing old readings.

4. [ ] **Intentional stop survives reopening.** Stop the whole `status-card` instance.
   Expect **Stopping → Stopped**, a successful operation, and `—` metrics—even if
   Details shows exit 137/143. Reopen Tandem: it still says **Stopped**.
   Start it again for the next test.

5. [ ] **Paused is distinct.** Copy `status-card/web`'s container ID from Details into
   `WEB_ID` below; run `docker pause "$WEB_ID"`, then Refresh.
   Expect amber **Paused**, memory retained, CPU `— · paused`.
   Run `docker unpause "$WEB_ID"` and Refresh: **Healthy** returns and CPU warms up.

   ```sh
   WEB_ID='paste the status-card web container ID here'
   ```

6. [ ] **Faults stay specific.** Run each row with a fresh instance, restoring settings
   afterwards; inspect children and operation details:

   | Template / instance | Setting | Expect |
   |---|---|---|
   | Guestbook / `status-unhealthy` | `FAIL_READINESS=1` | API red **Unhealthy**, instance **Degraded**, start operation failed; surviving API metrics appear after the operation ends. |
   | Greetings / `status-migration` | `FAIL_MIGRATION=1` | `migrate` red **Failed**, database still healthy, dependants waiting/not started; instance **Degraded**. |
   | Mailroom / `status-worker` | `FAIL_WORKER=1` | Worker **Stopped · exit 1** with warning; Redis remains healthy; instance **Degraded** with the worker reason. |

7. [ ] **Resource refresh + visibility.** On a healthy instance, press Refresh: memory
   updates first, CPU follows a second reading after a 1s pause.
   Leave the terminal focused for about 70s: sample age resets automatically.
   Collapse/filter the instance: parent totals still include it, without extra sampling.
   Leave the terminal unfocused for over a minute, then return: overdue readings refresh.
   With focus reporting, sustained unfocused polling is roughly every 5min.

8. [ ] **Running without a probe.** Save Postcard's template `compose.yaml`; temporarily
   replace `services.web.healthcheck` with `{"disable": true}` and create `status-running`.
   Expect blue **Running**, not green Healthy, even when its page loads.
   Restore the saved template immediately; existing containers keep their configuration.

## Cleanup

Restore all edited template files/settings. Unpause the test container if needed.
Delete only the `status-*` instances created here through Tandem; this permanently removes
their workspaces and volumes. Expect **Deleting**, then disappearance after cleanup completes.

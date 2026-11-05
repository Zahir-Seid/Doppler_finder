# Runbook

**Status:** Draft, v1. Commands assume the service binary and a SQLite volume;
they are updated when deployment is fixed (SDD section 11.4).

## Start and stop

1. Check the configuration in `config/default.toml` and the environment
   overrides.
2. Start the service under the process supervisor. The API and worker run in
   one process in v1.
3. Confirm `GET /v1/health` returns `ok` and `GET /v1/version` returns the
   expected build.
4. On SIGTERM the process stops claiming, drains running jobs, releases leases
   and exits. Do not send SIGKILL until the grace period has passed.

## Health checks

| Check | Healthy signal | Action |
| --- | --- | --- |
| API liveness | `/v1/health` returns `ok` | Restart if it fails twice |
| Queue depth | Stable or draining | Investigate a sustained rise |
| Lease expiry | No jobs repeatedly returning to `queued` | Check worker health and claim loop |
| Disk | Media and result volumes below the alert threshold | Clear expired data, add space |
| Figure rendering | Recent jobs have figures | Check the render stage and disk |
| Metrics | Prometheus scrape succeeds | Check the metrics endpoint and proxy |

## Common incidents

- **Jobs stuck in `running`:** wait for the lease to expire and the sweeper to
  requeue once. If the worker is unhealthy, restart it after draining. Record
  `worker_lost` outcomes.
- **Queue backlog:** confirm the worker is claiming, check CPU saturation and
  the per-job wall-time limit. Do not raise concurrency above the validated
  G5 level without a load test.
- **Media leak report:** follow `abuse-response.md`. Delete the job row and
  artifacts, then verify the retention sweeper.
- **Bad deploy:** stop the service, restore the previous binary, keep the
  database volume, and re-run the health checks. Migrations are forward-only;
  restore from backup if a migration must be undone.

## Logs and privacy

Logs carry `job_id` and `stage`, never media bytes, raw IP addresses, result
tokens or precise public-mode coordinates. Retention is 14 days by default.

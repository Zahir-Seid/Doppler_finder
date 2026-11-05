# Backups and restore

**Status:** Draft, v1. SQLite on a persistent volume is the v1 default; moving
to Postgres is a configuration change (D-4, D-5).

## What to back up

| Data | Frequency | Retention | Notes |
| --- | --- | --- | --- |
| SQLite database | Hourly snapshot plus daily full | 7 daily, 4 weekly | Use the SQLite backup API or a consistent file copy, never a live `cp` |
| Result artifacts and figures | Daily | 7 days | Rebuildable from the database and media, but expensive |
| Track cache | Not backed up | Rebuildable | Re-ingest from the recorded source and version |
| Configuration and migrations | On every change | Git history | Never store secrets in the repository |
| Secrets | On rotation | Per secret policy | Store in the deployment secret manager |

## Restore drill

1. Stop the service.
2. Restore the latest verified database snapshot to a scratch volume.
3. Run the health check against the restored copy with claiming disabled.
4. Compare job counts, the newest terminal job and one result manifest hash.
5. Record the restore time and any data loss window.
6. Repeat the drill quarterly and after a schema migration.

## Verify

- A backup is not valid until a restore has succeeded.
- Alert if the newest snapshot is older than two hours.
- Keep backups encrypted at rest and restrict access to the operator.

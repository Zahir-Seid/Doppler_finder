# Migrations

sqlx migrations arriving with the persistence task. They use the common SQL
subset so the same files run on SQLite in development and Postgres in
deployment (SDD section 9.6). Do not edit an applied migration; add a new one.

The initial schema is specified in SDD section 9.3: `jobs`, `job_events`,
`sessions`, `rate_limits`, `results` and the supporting indexes.

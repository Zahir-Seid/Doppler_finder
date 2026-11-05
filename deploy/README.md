# Deployment

Dockerfile, reverse proxy and service configuration arrive with v1
deployment (SDD section 11.4). The target is one small VM: reverse proxy for
TLS and request limits, the application binary, SQLite on a persistent volume,
track caches on the same or an attached disk, and a Prometheus scrape endpoint.

Secrets are injected by the deployment environment and never committed.

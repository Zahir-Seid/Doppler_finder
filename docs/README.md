# Documentation index

The PRD is authoritative for requirements. The SDD is authoritative for
architecture and method. The Rust Working Norms bind every line of code. This
folder also holds the working records that make progress auditable.

| File | Purpose | State |
| --- | --- | --- |
| [PRD.md](PRD.md) | Requirements, releases, gates, risks, decisions | Authoritative |
| [SDD.md](SDD.md) | Architecture, methods, data design, API, operations | Authoritative |
| [rust-working-norms.md](rust-working-norms.md) | Toolchain, lints, tests, definition of done | Authoritative |
| [traceability.csv](traceability.csv) | Requirement to section to test to status | Living |
| [validation-plan.md](validation-plan.md) | Datasets, gates and acceptance tests | Living |
| [questions.md](questions.md) | Open questions and their defaults | Living |
| [third_party.md](third_party.md) | Code and data licences | Living |
| [ui_ux.md](ui_ux.md) | v1 frontend specification | Draft, blocks v1 UI |
| [proposals/](proposals/) | Proposed changes to the PRD or SDD | Process |
| [ops/](ops/) | Runbook, abuse response, backups | Draft |

## Status vocabulary

| Status | Meaning |
| --- | --- |
| `not_started` | No implementation or test exists. |
| `scaffolded` | Types or interfaces exist; no requirement behaviour is tested. |
| `in_progress` | Implementation exists and tests are being written. |
| `done` | Requirement implemented and its test passes in CI. |
| `deferred_v3` | Explicitly out of scope until v3. |

## Rules

- Do not change the PRD or SDD directly. Write a proposal in `proposals/`.
- Record every ambiguity in `questions.md` and continue with the stated default.
- Update `traceability.csv` in the same pull request that adds or changes a
  requirement's implementation or test.
- Add every new crate, copied snippet and dataset to `third_party.md` before it
  enters the build.

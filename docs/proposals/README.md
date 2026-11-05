# Change proposals

The PRD and SDD are authoritative. Do not edit them directly. Propose a change
here, get it accepted, then update the source document and the traceability
row in the same pull request (Norms section 1).

## Process

1. Copy the template below to `NNNN-short-title.md`.
2. State the problem, the proposed change, the alternatives and the impact on
   requirements, tests and existing code.
3. Link the proposal from the pull request that would implement it.
4. Resolve it as `accepted`, `rejected` or `superseded`; keep the file.

## Template

```markdown
# NNNN: Short title

- **Status:** proposed
- **Date:** YYYY-MM-DD
- **Affects:** PRD section, SDD section, crate, requirement ID

## Problem

What is ambiguous, wrong or missing? Cite the line or requirement.

## Proposal

The exact change, including wording where possible.

## Alternatives

What else was considered and why it was rejected.

## Impact

Requirements, tests, migration, compatibility and schedule.

## Decision

Filled in when accepted or rejected, with the date and owner.
```

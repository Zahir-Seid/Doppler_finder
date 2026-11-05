# UI and UX specification

**Status:** Draft. Blocks the v1 frontend. The PRD promises a Nuxt/bun
frontend; this document defines what it must do before any page is built.

## Scope

v1 is a defensive audit: a person uploads a recording and learns how locatable
it is, with the evidence and uncertainty behind the answer. There are no
accounts. The public result is deliberately coarse (decision D-1, FR-OUT-03).

## Pages

| Route | Purpose | Required states |
| --- | --- | --- |
| `/` | Explain the audit, upload media, choose a coarse region and date window, optional known start time, attestation | Empty, validating, uploading, rejected |
| `/jobs/{token}` | Live progress over SSE, cancel, then the result | Queued, running stage, done, failed, expired, cancelled |
| `/jobs/{token}/report` | Figures and evidence for a completed result | Locatable, ambiguous, not locatable, partial evidence |

## Result presentation

- **Locatable:** show the region cell, the confidence level, the matched flight
  summary, the per-event quality and the strongest rival. Never show the
  precise ellipse or exact coordinates in public mode.
- **Ambiguous:** show the ranked alternatives and the margin over the best
  rival. Explain which clue would separate them.
- **Not locatable:** this is a successful result, not an error. State the
  reason codes and what would improve the answer (for example a second
  flyover, a known start time or a longer clip).
- Always show the run manifest hash and a link to the figures.

## Copy and privacy

- Say "region" and "confidence", not "exact location", unless research mode is
  enabled.
- Do not log or display raw IP addresses, result tokens or precise coordinates.
- Explain that media is deleted within one hour of job end and results within
  24 hours.
- State plainly that a single event without a known time is refused.

## Interaction rules

- Enforce the public limits before reading the upload body: 25 MB and the
  provisional 90 s cap.
- Require the ownership or permission attestation (FR-AC-05).
- Use an anonymous, unguessable session cookie; no account UI in v1.
- Make cancel idempotent and show the terminal state.
- Reconnect SSE with `Last-Event-ID` and replay missed events.
- Show stable error codes from SDD 10.3 with a human sentence and a retry hint.

## Accessibility

- WCAG 2.2 AA: keyboard-operable upload and cancel, visible focus, labels tied
  to inputs, contrast at least 4.5:1 for body text.
- Do not encode the verdict in colour alone; use text and shape.
- Respect `prefers-reduced-motion` for progress and map animation.
- The map region must have a text equivalent with the cell bounds and
  confidence.

## Performance

- First meaningful paint under 2 s on a mid-range phone over 4G.
- Progress updates at most once per second and at least once per stage change.
- Figures lazy-load after the text verdict.

## Open questions

1. Nuxt major version and bun pin (Q-6).
2. Map provider and tile licence; must be recorded in `third_party.md`.
3. Bot-challenge provider and its privacy terms (FR-AC-03).
4. Final public copy for the `Locatable` versus 2 km cell distinction (Q-5).
5. Whether the demo cap changes after the v0 two-flyover count (Q-1).

## Acceptance

- A first-time user can upload, understand the verdict and find the evidence
  without reading the SDD.
- A `Not locatable` result never looks like a system failure.
- No page exposes precise coordinates or media bytes in public mode.
- Keyboard-only, screen-reader and reduced-motion checks pass.

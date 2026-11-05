# Abuse response

**Status:** Draft, v1.

## Intake

- Provide one contact address on the site and in the repository security
  policy.
- Record the report date, reporter, job token if supplied, and the alleged
  harm. Do not ask for the media unless it is necessary and consented.

## Triage

| Class | Examples | First action |
| --- | --- | --- |
| Privacy | A recording was analyzed without consent | Delete the job and artifacts, confirm deletion, preserve only non-content metadata |
| Harassment | Targeted location inference | Delete, block the session, review rate limits |
| Illegal content | Media that must not be processed | Stop processing, preserve legally required evidence, follow local law |
| Service abuse | Scraping, queue flooding, challenge bypass | Tighten limits, block the source, review the global budget |
| Security | Token guessing, injection, leak | Follow the security policy, rotate secrets, patch |

## Containment

1. Disable the affected job or session.
2. Delete media immediately; delete results if they contain precise data.
3. Preserve only logs needed for the investigation, with personal data
   minimized.
4. If a secret or token leaked, rotate it and invalidate outstanding result
   tokens.
5. Record the incident and the fix in the operations log.

## After

- Update the rate limits, challenge or copy that allowed the abuse.
- Add a regression test or monitoring signal.
- Tell the reporter the outcome within the stated window.

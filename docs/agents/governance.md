# Governance: journal tags, state table, Team trailer

## Journal tags

Entries in `docs/journal/` use one tag, at most 8 lines, first line a UTC timestamp and
the team (CL or CX):

- `[CONTRACT-PUBLISHED]` a frozen or draft contract digest is published.
- `[CONTRACT-CHANGE]` a published contract changes; new versioned file, never in place.
- `[ASK]` a one-sentence decision for the user, with a needed-by date and a default
  only where the default is safe. Silence is never approval.
- `[DEP-ASK]` a request to another lane to change a file it owns (see OWNERS.md).
- `[BLOCKED]` work cannot continue; names the blocker.
- `[DONE]` carries target, sha, `doubles[]`, agent-hours, tests added, gate minutes.
- `[HANDOFF]` state passed to another actor.
- `[FINDING]` a verified observation that others may rely on.

## State table

Each session starts with a 10-row state table (WP, lane, state, last sha, blocker)
at the top of its journal entry.

## Team trailer

Every commit message ends with a `Team:` trailer naming the team (`Team: CL` or
`Team: CX`) next to the usual co-author line. Ownership is decided by OWNERS.md,
never by the trailer.

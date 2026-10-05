# 0655 W13 prompt patches and new agents announceable (UTC 2026-10-05T10:00Z, CLAUDE) [DONE]

Lane W13, branch `claude/w13-prompt-and-agent-announce` (from w11; no PR, no merge dependency).
- Prompt patch: native control run (`native_binding`), harness wording probe with real samples (`sample_probes.py`), honest `coverage` in verdict story and ES/PT dossier. Needs agent-core PR 50 (local scratch merge only) for `candidate_bound`.
- New agent: suite generator, full donor closure (`registry_writer::closure`), donor release settings for evaluation only (labelled assumption, never delivered); recepcion routing = human follow-up, not measured.
- Live on own stack: (i) prompt patch announced, (ii) text-identical prompt not_fixed, (iii) new agent announced. Blocker carried: agent-core requires the admin role to draft release interrupts, so the evaluation drafts used a local admin stand-in.
- Fixed: value_loop test double (closure reads), live nonce vs idempotent replay. See docs/dev/W13_PROMPT_AND_AGENT_ANNOUNCE.md.

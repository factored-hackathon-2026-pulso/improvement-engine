# Independent adversarial review — llm-gateway PR #4

- PR: https://github.com/pulso-factored/llm-gateway/pull/4
- Reviewed head: `bdd9a9d8dd568d6322379722ae10285da9b77723`
- State at review: open, non-draft, mergeable (last checked 2026-10-05); no submitted reviews or discussion comments were returned by the connector.
- Method: read PR metadata/body, changed-file inventory, ADR, gateway telemetry code/tests and Agent Core data/telemetry constraints from Pulso V3. No provider call or external telemetry endpoint was used.

## Findings

### BLOCKER for use with real customer or bank data — raw caller-controlled identity/tags are exported even with content capture off

**Location:** `internal/gateway/langfuse.go:31-36,47-66`.

The new code unconditionally maps inbound baggage `user.id` to `langfuse.user.id` and copies up to 16 caller-controlled values from `langfuse.trace.tags` (each capped only by byte length). The “closed” baggage allowlist constrains key names, not the values. An upstream caller can place an email, customer identifier, phone, account reference, or free text in those approved baggage slots, and the span exports it even when `LLM_GATEWAY_TRACE_CONTENT` is unset. This conflicts with Pulso V3's rule that direct PII does not appear in prompts/logs/traces.

**Failing-test sketch:** extract baggage `user.id=synthetic.person@example.test` and `langfuse.trace.tags=customer:5551234567`; call Generate with content capture disabled; assert neither raw value appears in any ended span attribute or exported OTLP payload. The current code is expected to fail. Define allowed opaque IDs or redact/hash under a documented stable policy before emission.

### BLOCKER before enabling content capture on any mixed or real-data deployment — global opt-in stores unfiltered prompts and outputs

**Location:** `internal/gateway/langfuse.go:97-121`; `internal/gateway/gateway.go:120-127,157-170`; `docs/adr/0002-langfuse-attributes-and-opt-in-content-capture.md:12-21`.

When `LLM_GATEWAY_TRACE_CONTENT=1`, the gateway records the exact system and user messages and successful output (duplicated into both Langfuse and GenAI attributes), with truncation as the only content control. This is a process-wide environment switch, not a consumer/purpose/data-class authorization or a redaction boundary. The ADR recommends a masking collector but does not enforce one. A 32 KiB cap limits size; it does not remove PII, secrets included in prompts, or confidential model output. The PR's claim that an owning consumer authorized sending content is documentation, not an enforced contract for all gateway consumers.

**Failing-test sketch:** with content capture enabled, supply entirely synthetic canary PII/secret strings in `Request.Inputs`, system prompt and model output; assert the emitted OTLP span contains only approved/redacted content, or that the request is denied unless the consumer, purpose, and endpoint are explicitly authorized. Also test that another consumer cannot inherit the process-wide setting accidentally. Current tests assert content is captured exactly, so they demonstrate the exposure rather than its safety.

## Positive controls observed

- Content capture defaults off and has a per-field byte cap; provider error text, headers, credentials and failed-call outputs are excluded.
- Arbitrary baggage keys are not copied wholesale, and trace tags are count/size bounded.
- Existing W3C trace context is preserved and tests cover parent linkage.

## Disposition

Default-off is a useful safeguard but does not make the new attributes safe for mixed consumers. Do not enable this for real traffic until user identity/tag values are governed and content capture is consumer-scoped with an enforced redaction/authorization boundary. No comments, fixes, or provider tests were posted.


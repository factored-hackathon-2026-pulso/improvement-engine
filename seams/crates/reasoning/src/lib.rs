//! Reasoning roles (L2): Scout, Verifier and Builder on the engine `ModelPort`.
//!
//! A corroborated cells signal (output of `steps_cli cells`, semantics `claude-standin`) becomes
//! 1. an OPPORTUNITY (Scout: a hypothesis about which real agent-core artifact carries the mechanism),
//! 2. an independent VERIFICATION (a separate port and prompt that sees only the claim and the treated evidence, plus a
//!    deterministic recompute the model cannot overrule), and
//! 3. a concrete PROPOSAL (Builder) over REAL artifact ids, as an ANCHORED PATCH that `patch::compile` applies byte-exact to
//!    the base text. The Builder never rewrites a whole text: it names an anchor id from a menu the engine computed (protected
//!    clauses are never on the menu) and writes only the replacement fragment. A new specialist agent is the second proposal kind
//!    (closure copy of the donor `consultas`).
//!
//! The model proposes, code disposes: every answer is schema-checked, every id must resolve, every text must be clean of PII-like
//! patterns, protected clauses and locale parity are re-checked on the compiled result. A failure is a typed stop, never a silent
//! fallback to another port. The model only ever sees TREATED aggregates (TPS scan in the port); derived aggregates of real
//! data reach a hosted model only with the explicit opt-in flag (`pipeline::Opts::allow_derived_aggregates`).
//!
//! Honest labels: every report carries `doubles[]` entries for every model call that was not an answered gateway call, the
//! artifact baseline is labelled `fixture-baseline` (agent-core e2e seed, not a live registry), and the sensor stays
//! `claude-standin`.
pub mod art2;
pub mod catalog;
pub mod dossier;
pub mod finding;
pub mod flow_edits;
pub mod live;
pub mod mapping;
pub mod patch;
pub mod pipeline;
pub mod roles;
pub mod rubric;
pub mod testkit;

pub use finding::{Finding, Source};

/// Text a model wrote (hypothesis, replacement, routing summary): bounded, and free of anything PII-like or token-like.
/// Digit runs of 4 or more are refused too (the Core PII wrapper tokenises runs of 6+, and no proposal text needs a figure).
pub fn clean_text(s: &str, max_chars: usize) -> Result<(), String> {
    if s.trim().is_empty() {
        return Err("empty text".into());
    }
    if s.chars().count() > max_chars {
        return Err(format!("longer than {max_chars} characters"));
    }
    if s.contains('\u{27e6}') || s.contains('\u{27e7}') {
        return Err("contains a PII token marker".into());
    }
    if s.contains('@') {
        return Err("contains an at-sign (email-like)".into());
    }
    let mut run = 0usize;
    for c in s.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= 4 {
                return Err("contains a digit run of 4 or more".into());
            }
        } else {
            run = 0;
        }
    }
    if s.chars().any(|c| !c.is_ascii() && c.is_numeric()) {
        return Err("contains a non-ASCII numeral".into());
    }
    Ok(())
}

/// An at-sign that is not part of an `id@version` reference (`obtener_pqr@1`, `flow:x@1.0.1`).
pub fn email_like(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).any(|i| b[i] == b'@' && !b.get(i + 1).is_some_and(u8::is_ascii_digit))
}

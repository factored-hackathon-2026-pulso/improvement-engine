//! Commit record codec for the executor (E2), compatible with run_once (E1) records.
//! `P <payload>`, optional `F <effect>` (E2), `E <event>` lines. E1 records carry no F line (= NoEffect).
use abi::{EffectState, OutputEnvelope};

pub(crate) fn effect_name(e: EffectState) -> &'static str {
    match e {
        EffectState::NoEffect => "NoEffect",
        EffectState::DispatchBegun => "DispatchBegun",
        EffectState::UnknownPendingReconciliation => "UnknownPendingReconciliation",
        EffectState::AppliedAcknowledged => "AppliedAcknowledged",
        EffectState::CompletedNoEffect => "CompletedNoEffect",
        EffectState::CancelledBeforeEffect => "CancelledBeforeEffect",
    }
}

fn effect_of(s: &str) -> EffectState {
    // unknown text is treated as the blocking state: never skip what we cannot read
    match s {
        "NoEffect" => EffectState::NoEffect,
        "DispatchBegun" => EffectState::DispatchBegun,
        "AppliedAcknowledged" => EffectState::AppliedAcknowledged,
        "CompletedNoEffect" => EffectState::CompletedNoEffect,
        "CancelledBeforeEffect" => EffectState::CancelledBeforeEffect,
        _ => EffectState::UnknownPendingReconciliation,
    }
}

pub(crate) fn encode_full(out: &OutputEnvelope) -> String {
    let mut s = format!("P {}\nF {}", out.payload, effect_name(out.effect));
    for e in &out.events {
        s.push_str(&format!("\nE {e}"));
    }
    s
}

pub(crate) fn decode_full(rec: &str) -> (String, Vec<String>, EffectState) {
    let mut payload = String::new();
    let mut events = vec![];
    let mut effect = EffectState::NoEffect;
    for l in rec.lines() {
        if let Some(p) = l.strip_prefix("P ") {
            payload = p.to_string();
        } else if let Some(f) = l.strip_prefix("F ") {
            effect = effect_of(f);
        } else if let Some(e) = l.strip_prefix("E ") {
            events.push(e.to_string());
        }
    }
    (payload, events, effect)
}

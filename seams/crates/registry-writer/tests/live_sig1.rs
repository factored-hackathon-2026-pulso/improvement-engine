//! SIG1 LIVE (ignored): `resolve_links` over real HTTP against the REAL support-platform backend (`uv run cc-api` from the platform's
//! backend directory, a freshly recreated `cc_platform.db`, throwaway service token in the process environment).
//! Env (process environment only, never printed): PULSO_PLATFORM_URL, PULSO_PLATFORM_SERVICE_TOKEN.
//! Run: `cargo test -j 1 -p registry-writer --test live_sig1 -- --ignored --nocapture`.
//! Expectations reflect the platform's demo seed (17 cases, 15 of them `es`; every single channel / case type cell is below k = 10).
mod common;
use common::finding;
use core_client::authorizer::Jws;
use registry_writer::announce::{Links, parse_platform_url, resolve_links, valid_case_id};
use registry_writer::transport::HttpTransport;
use std::time::Duration;

fn cell(dims: &[(&str, &str)]) -> reasoning::finding::Finding {
    let mut f = finding();
    f.dims = dims.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    f
}

#[test]
#[ignore = "needs the live platform (uv run cc-api with CC_INTERNAL_SERVICE_TOKEN set)"]
fn live_evidence_route_resolves_real_case_ids_and_degrades_honestly() {
    let url = std::env::var("PULSO_PLATFORM_URL").expect("PULSO_PLATFORM_URL");
    let token = Jws::new(std::env::var("PULSO_PLATFORM_SERVICE_TOKEN").expect("PULSO_PLATFORM_SERVICE_TOKEN"));
    let t = HttpTransport::new(&parse_platform_url(&url).unwrap(), Duration::from_secs(15));

    // a cell with at least k cases: real ids, `CASE-` shape, at most 8
    let real = resolve_links(&t, &token, &cell(&[("language", "es")]));
    println!("language=es -> {}", real.record());
    match &real {
        Links::Real(ids) => {
            assert!(!ids.is_empty() && ids.len() <= 8 && ids.iter().all(|i| valid_case_id(i)), "{ids:?}");
            println!("  {} real ids, first={} (shape verified, newest first)", ids.len(), ids[0]);
        }
        other => panic!("expected real ids, got {other:?}"),
    }
    // cells below k (single channel, language pt, case type): the platform answers suppressed, the engine keeps the opaque link
    for dims in [&[("channel", "web_chat")][..], &[("language", "pt")], &[("case_type", "undue_charge")], &[("channel", "web_chat"), ("language", "es")]] {
        let l = resolve_links(&t, &token, &cell(dims));
        println!("{dims:?} -> {}", l.record());
        assert_eq!(l, Links::Opaque("suppressed_below_k"), "{dims:?}");
    }
    // a wrong token is 401, a cell the platform cannot express is never sent
    let wrong = resolve_links(&t, &Jws::new("not-the-token".into()), &cell(&[("language", "es")]));
    println!("wrong token -> {}", wrong.record());
    assert_eq!(wrong, Links::Opaque("unauthorized"));
    let unmappable = resolve_links(&t, &token, &finding());
    println!("bank reason x channel cell -> {}", unmappable.record());
    assert_eq!(unmappable, Links::Opaque("unmappable_dimension"));
}

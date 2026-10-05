//! The engine's closed allow-list. Anything else (approve, publish, promote, revoke, reject, reopen, alias writes, release reads
//! that are not needed) is refused BEFORE the transport: the engine proposes, agent-core manages. W11 adds exactly two management
//! verbs, `freeze` and `evaluate`, because the engine proves a proposal (regression suite fails on the base, passes on the
//! candidate) before announcing it; both only ever act on a MANUAL-origin evaluation draft and neither releases anything. The human
//! decisions (approve, publish, promote, reject) stay refused.

/// ANN1: the platform client's allow-list is ONE route, the engine announcement. Everything else on the platform is refused.
pub fn platform_allowed(method: &str, path: &str) -> bool {
    if method == "GET" {
        return evidence_query_allowed(path);
    }
    method == "POST" && path == "/api/v1/internal/builder/proposals/announce"
}

/// SIG1: the one read the engine makes on the platform, `GET /api/v1/internal/evidence/cases?<dims>` (which real cases sit in a
/// cell). Exactly that path and a query of `key=value` pairs over a closed charset (no encoding, no fragment, no second path).
fn evidence_query_allowed(path: &str) -> bool {
    const ROUTE: &str = "/api/v1/internal/evidence/cases";
    let Some(q) = path.strip_prefix(ROUTE) else { return false };
    let Some(q) = q.strip_prefix('?') else { return q.is_empty() };
    !q.is_empty()
        && q.split('&').all(|kv| {
            matches!(kv.split_once('='), Some((k, v)) if !k.is_empty() && !v.is_empty() && k.bytes().all(|b| b.is_ascii_alphabetic()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.')))
        })
}

/// `true` only for the exact operations of the writer.
pub fn allowed(method: &str, path: &str) -> bool {
    // Space, control byte or fragment: the path is written verbatim into the HTTP request line. The ONE query the engine sends is the
    // proposal listing with a fixed shape (below); every other `?` is refused.
    if !path.bytes().all(|b| b.is_ascii_graphic() && b != b'#') {
        return false;
    }
    let (p, query) = match path.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path, None),
    };
    if let Some(q) = query {
        return method == "GET" && p == "/v1/registry/proposals" && listing_query(q);
    }
    if p == "/v1/runs" {
        return method == "POST";
    }
    let Some(rest) = p.strip_prefix("/v1/registry/") else { return false };
    let seg: Vec<&str> = rest.split('/').collect();
    match (method, seg.as_slice()) {
        ("POST", ["proposals"]) => true,
        ("GET", ["proposals", id]) => ok_seg(id),
        ("PUT", ["proposals", id, "draft"]) => ok_seg(id),
        ("POST", ["proposals", id, "validate"]) => ok_seg(id),
        ("POST", ["proposals", id, "freeze"]) => ok_seg(id),
        ("POST", ["proposals", id, "evaluate"]) => ok_seg(id),
        // INH1: the donor's published release (alias -> release id, then the release itself); read-only, nothing else of releases.
        ("GET", ["aliases", agent, alias]) => ok_seg(agent) && ok_seg(alias),
        ("GET", ["releases", rid]) => ok_seg(rid),
        ("GET", ["entities", kind, tail @ ..]) => ok_seg(kind) && !tail.is_empty() && tail.iter().all(|s| ok_seg(s)),
        _ => false,
    }
}

/// E8: the ONE extra operation, `POST /v1/registry/proposals/{id}/reopen`, used only by `eval::close_scratch` on the engine's own
/// evaluation scratch (never through `allowed`, which keeps refusing `reopen` like every other decision verb).
pub fn scratch_close_allowed(method: &str, path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/v1/registry/proposals/") else { return false };
    method == "POST" && matches!(rest.split('/').collect::<Vec<_>>().as_slice(), [id, "reopen"] if ok_seg(id))
}

/// A path segment: ids, kinds and versions only (no traversal, no encoded separators).
pub fn ok_seg(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.len() <= 120 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
}

/// `agent_id=<id>&limit=<1-3 digits>[&offset=<1-6 digits>]`: nothing else may travel in a query.
fn listing_query(q: &str) -> bool {
    let parts: Vec<&str> = q.split('&').collect();
    let num = |v: &str, n: usize| !v.is_empty() && v.len() <= n && v.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [a, l] | [a, l, _] => {
            a.strip_prefix("agent_id=").is_some_and(ok_seg)
                && l.strip_prefix("limit=").is_some_and(|v| num(v, 3))
                && parts.get(2).is_none_or(|o| o.strip_prefix("offset=").is_some_and(|v| num(v, 6)))
        }
        _ => false,
    }
}

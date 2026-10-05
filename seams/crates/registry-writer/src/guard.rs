//! The engine's closed allow-list. Anything else (approve, publish, promote, revoke, reject, freeze, evaluate, reopen, alias writes,
//! release reads that are not needed) is refused BEFORE the transport: the engine proposes, agent-core manages.

/// `true` only for the exact operations of the writer.
pub fn allowed(method: &str, path: &str) -> bool {
    let p = path.split('?').next().unwrap_or("");
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
        ("GET", ["entities", kind, tail @ ..]) => ok_seg(kind) && !tail.is_empty() && tail.iter().all(|s| ok_seg(s)),
        _ => false,
    }
}

/// A path segment: ids, kinds and versions only (no traversal, no encoded separators).
pub fn ok_seg(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.len() <= 120 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
}

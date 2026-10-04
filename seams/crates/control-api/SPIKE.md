# E9s spike: tiny_http SSE, disconnect detection, memory (verdict: tiny_http holds; do NOT add axum/tokio)

Method: `cargo run --release --example sse_spike -- <heartbeat_ms> <streams>` (Windows 11, release build, one process:
tiny_http server + raw `TcpStream` clients). SSE is written on the raw writer from `Request::into_writer`
(`src/sse.rs`, `Connection: close`, flush per event); one OS thread per stream, 128 KiB stack. A write error is the
disconnect signal, so a heartbeat comment bounds detection (peer FIN: first write succeeds and draws an RST, the
second write fails => detection <= 2 heartbeats).

| number | heartbeat 1000 ms | heartbeat 500 ms |
|---|---|---|
| disconnect detection (clean client close), 6 samples, phase varied | max 1.86 s (1.18-1.86) | max 0.95 s (0.59-0.95) |
| memory per open stream (working-set delta) | 46.7 KiB at 500 streams (5.7 -> 28.5 MiB) | 47.2 KiB at 100 streams |

Kill criterion was "disconnect not detected within 5 s": passed with margin (1.86 s at the 1 s default; tune the
heartbeat for tighter bounds, 0.95 s at 500 ms). The memory figure includes the client sockets of the harness, so it
is an upper bound for the server side.

axum/tokio estimate (NOT measured; documented only): hyper notices a peer close through the connection read half
and drops the body stream almost immediately (order of 10-100 ms, no heartbeat needed); a task plus socket costs
roughly 4-16 KiB per stream versus ~47 KiB here (thread stack commit). Scale limit: tiny_http is thread-per-stream,
fine for hundreds of console/operator feeds (the control-api LITE audience), not for tens of thousands.

Verdict: tiny_http is sufficient for control-api LITE SSE. Revisit axum only if the number of concurrent feeds exceeds
~2000 or sub-second disconnect detection without heartbeats becomes a requirement. No tokio dependency added.

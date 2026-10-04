//! Debug run-event feed (DebugApi `openStream`, `debug-console/src/api/debug/stream.ts`).
//! `GET /internal/v1/debug/runs/{run}/stream?after_sequence=N` (+ `Last-Event-ID`, the larger wins) resumes after a cursor.
//! An unknown cursor (beyond the head) or a purged one (below the floor) is `410 cursor_expired` with
//! `recovery: {snapshot_ref, after_sequence}`: the snapshot ref stays inside the run, nothing is invented for the purged
//! range. A run of another tenant is a 404 (indistinguishable from unknown). Auth: service JWT, aud `control-api`, scope
//! `debug_read`; the stream itself is written by `server::stream` and is bounded in age, so a token is re-presented on resume.
//! Run events are seeded through the admin channel until the engine writes them (`run_events`: tenant, run, floor, events).
use crate::app::{App, Req, Resp, code, json_resp};
use crate::auth::Expect;
use serde_json::{Value, json};

pub struct StreamPlan {
    pub tenant: String,
    pub run: String,
    pub after: i64,
}

fn cursor(s: &str) -> Option<i64> {
    s.trim().parse::<i64>().ok().filter(|n| *n >= 0)
}

impl App {
    /// `None`: not a stream route. `Some(Err)`: answer this response. `Some(Ok)`: the server should stream.
    pub fn stream_route(&self, r: &Req) -> Option<Result<StreamPlan, Resp>> {
        let run = r.path.strip_prefix("/internal/v1/debug/runs/")?.strip_suffix("/stream")?;
        if r.method != "GET" || run.is_empty() || run.contains('/') {
            return None;
        }
        Some(self.stream_open(r, run))
    }

    fn stream_open(&self, r: &Req, run: &str) -> Result<StreamPlan, Resp> {
        let claims = self.authn(&self.control, r, &Expect { aud: "control-api", scope: Some("debug_read"), purpose: None }).map_err(Self::denied)?;
        let tenant = claims["tenant_id"].as_str().unwrap_or_default().to_string();
        let mut after = 0;
        for raw in [r.query.split('&').find_map(|kv| kv.strip_prefix("after_sequence=")), r.headers.get("last-event-id").map(String::as_str)].into_iter().flatten() {
            after = after.max(cursor(raw).ok_or_else(|| code(400, "bad_cursor"))?);
        }
        let doc = self.store.get_doc("run_events", &tenant, run).ok_or_else(|| code(404, "run_not_found"))?;
        let floor = doc["floor"].as_i64().unwrap_or(0);
        let head = floor + doc["events"].as_array().map_or(0, |e| e.len() as i64);
        if after < floor || after > head {
            let body = json!({"code": "cursor_expired", "message": "cursor outside the retained range", "retryable": false,
                              "recovery": {"snapshot_ref": format!("/internal/v1/debug/runs/{run}/graph"), "after_sequence": if after < floor { floor } else { head }}});
            return Err(json_resp(410, body));
        }
        Ok(StreamPlan { tenant, run: run.to_string(), after })
    }

    /// Events with `sequence > after`, in order.
    pub fn run_events_after(&self, tenant: &str, run: &str, after: i64) -> Vec<Value> {
        let Some(doc) = self.store.get_doc("run_events", tenant, run) else { return Vec::new() };
        doc["events"].as_array().into_iter().flatten().filter(|e| e["sequence"].as_i64().is_some_and(|s| s > after)).cloned().collect()
    }

    /// Admin seeding / append: sequences must continue the retained log without holes.
    pub(crate) fn seed_run_events(&self, item: &Value) -> Result<(), Resp> {
        let (Some(tenant), Some(run), Some(events)) = (item["tenant"].as_str(), item["run"].as_str(), item["events"].as_array()) else { return Err(code(422, "schema_invalid")) };
        let mut doc = self.store.get_doc("run_events", tenant, run).unwrap_or_else(|| json!({"floor": item["floor"].as_i64().unwrap_or(0), "events": []}));
        let mut next = doc["floor"].as_i64().unwrap_or(0) + doc["events"].as_array().map_or(0, |e| e.len() as i64) + 1;
        for e in events {
            if e["sequence"].as_i64() != Some(next) || e["run_ref"] != run {
                return Err(code(422, "sequence_not_contiguous"));
            }
            doc["events"].as_array_mut().expect("array").push(e.clone());
            next += 1;
        }
        self.store.put_doc("run_events", tenant, run, doc);
        Ok(())
    }
}

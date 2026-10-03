// Replays the demo run live over SSE: switches the fixture server to the `demo_replay` world (all nodes planned) and emits each node transition.
// usage: node demo/serve/play.mjs <final-world.json> [base-url] [delay-ms]
import fs from 'node:fs';

const [worldFile, base = 'http://127.0.0.1:4010', delay = '1200'] = process.argv.slice(2);
const world = JSON.parse(fs.readFileSync(worldFile, 'utf8'));
const post = (p, body) => fetch(base + p, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }).then((r) => r.json());
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
console.log(await post('/__fixture/scenario', { scenario: 'demo_replay' }));
await sleep(Number(delay));
for (const [runId, run] of Object.entries(world.runs)) {
  for (const n of run.nodes) {
    if (n.status === 'planned') continue;
    await post('/__fixture/emit', { run_id: runId, node_id: n.node_id, status: 'running' });
    await sleep(Number(delay) / 2);
    await post('/__fixture/emit', { run_id: runId, node_id: n.node_id, status: n.status });
    await sleep(Number(delay));
  }
}
console.log('replay done');

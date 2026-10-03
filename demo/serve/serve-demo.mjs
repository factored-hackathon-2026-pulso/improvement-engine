// DEPRECATED: use FIXTURE_WORLD_FILE=demo/out/world.json node debug-console/fixture-server/server.mjs (see demo/README.md).
// Serves the debug-console fixture API/SSE with the demo world (no console file is modified).
// usage: node demo/serve/serve-demo.mjs <world.json> [replay-world.json]   (env FIXTURE_PORT, default 4010)
// It rewrites a COPY of debug-console/fixture-server/server.mjs (3 anchored replacements, fails loudly if the console drifted).
import fs from 'node:fs';
import path from 'node:path';
import url from 'node:url';

const here = path.dirname(url.fileURLToPath(import.meta.url));
const repo = path.resolve(here, '..', '..');
const [worldFile, replayFile] = process.argv.slice(2);
if (!worldFile) { console.error('usage: serve-demo.mjs <world.json> [replay-world.json]'); process.exit(2); }
const outDir = path.resolve(here, '..', 'out');
fs.mkdirSync(outDir, { recursive: true });
const scenariosUrl = url.pathToFileURL(path.join(repo, 'debug-console', 'fixtures', 'scenarios.mjs')).href;
const read = (f) => JSON.stringify(JSON.parse(fs.readFileSync(path.resolve(f), 'utf8')));
const shim = `import { makeScenario as base } from '${scenariosUrl}';
const DEMO = ${read(worldFile)};
const REPLAY = ${replayFile ? read(replayFile) : 'null'};
export const makeScenario = (name) => name === 'demo' ? structuredClone(DEMO) : name === 'demo_replay' && REPLAY ? structuredClone(REPLAY) : base(name);
`;
fs.writeFileSync(path.join(outDir, 'scenario-shim.mjs'), shim);
let src = fs.readFileSync(path.join(repo, 'debug-console', 'fixture-server', 'server.mjs'), 'utf8');
const rep = (from, to) => { if (!src.includes(from)) { console.error(`console fixture-server drifted, anchor missing: ${from}`); process.exit(3); } src = src.replace(from, to); };
rep("import { makeScenario } from '../fixtures/scenarios.mjs';", `import { makeScenario } from '${url.pathToFileURL(path.join(outDir, 'scenario-shim.mjs')).href}';`);
rep("let scenario = 'default';", "let scenario = 'demo';");
rep("target: 'mock', runtime_profile: 'fixture', doubles: ['api_fixture', 'sse_fixture', 'identity_fixture'], pin: null",
  "target: 'demo-standin', runtime_profile: world.demo?.runtime_profile ?? 'fixture', doubles: world.demo?.doubles ?? ['api_fixture'], pin: null");
const out = path.join(outDir, 'server.generated.mjs');
fs.writeFileSync(out, src);
await import(url.pathToFileURL(out).href);

// node capture.mjs <console base url> <run id> <out dir>
// Screenshots every panel of a run in a headless Chromium (Playwright from debug-console/node_modules). Needs a running `pulso serve`.
import { chromium } from '../../../../debug-console/node_modules/@playwright/test/index.mjs';

const [base, run, out] = process.argv.slice(2);
if (!base || !run || !out) { console.error('usage: node capture.mjs <base> <run> <outdir>'); process.exit(2); }
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
await page.goto(`${base}/#/run/${run}`);
await page.getByTestId('investigation').waitFor();
await page.getByTestId('decision').waitFor();
await page.waitForTimeout(1500);
const panels = [['trace-panel', '01-trazas'], ['investigation', '02-investigacion'], ['alternatives', '03-alternativas'], ['gates', '04-gates'], ['diff', '05-diff'], ['decision', '06-decision']];
for (const [id, name] of panels) {
  await page.getByTestId(id).screenshot({ path: `${out}/${name}.png` });
  console.log('saved', name, (await page.getByTestId(id).innerText()).split('\n').slice(0, 3).join(' | '));
}
await page.screenshot({ path: `${out}/00-pagina-completa.png`, fullPage: true });
await browser.close();

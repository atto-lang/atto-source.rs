// Real Rust/Wasm browser-worker integration, independently run in Firefox and
// WebKit (Safari's rendering/JS engine). Never substitutes a JS compiler.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {firefox, webkit} from 'playwright';

const engines = {firefox, webkit};
const names = (process.env.ATTO_BROWSER_ENGINES ?? 'firefox,webkit').split(',');
assert.ok(names.length > 0 && names.every(n => n in engines), 'unknown browser engine');

const entries = new Map([
  ['/index.html', [new URL('./browser/index.html', import.meta.url), 'text/html; charset=utf-8']],
  ['/worker.mjs', [new URL('./browser/worker.mjs', import.meta.url), 'text/javascript; charset=utf-8']],
  ['/atto_compiler.wasm', [new URL('../target/wasm32-unknown-unknown/release/atto_compiler.wasm', import.meta.url), 'application/wasm']],
]);
const requests = [];
const server = createServer(async (req, res) => {
  requests.push([req.method, req.url]);
  const item = entries.get(req.url);
  if (req.method !== 'GET' || !item) {
    res.writeHead(404);
    res.end('not found');
    return;
  }
  try {
    res.writeHead(200, {
      'content-type': item[1],
      'x-content-type-options': 'nosniff',
      'cache-control': 'no-store',
    });
    res.end(await readFile(item[0]));
  } catch {
    res.writeHead(404);
    res.end('missing artifact');
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const url = 'http://127.0.0.1:' + server.address().port + '/index.html';

const fixture = await readFile(new URL('./fixtures/pass/107-playground-trap-expression.atto', import.meta.url), 'utf8');
const previewStart = fixture.indexOf('// SYNTAX PREVIEW');
assert.ok(previewStart > 0, 'the exact public preview was not included');
const program = fixture.slice(previewStart);
const sourceBlob = createHash('sha1')
  .update('blob ' + Buffer.byteLength(program, 'utf8') + '\0')
  .update(program).digest('hex');
assert.equal(sourceBlob, '71d22c2365c687fbce044cea7d7d2927c3fb6e86');

const tests = [
  ['actual public trap preview', program,
    'main() -> 7', /^5\nid must be nonnegative\nan ordinary throw was caught\nmain\(\) -> 7$/],
  ['panic bypasses trap',
    'pub fnc main(): int { val n: int = 9223372036854775807; match trap (n + 1) over on Some(v) -> { return v; } on None -> { return 0; } end }',
    'E_PANIC', /E_PANIC: integer overflow/],
  ['raw non-Error throw fails compilation',
    'pub fnc main(): int { throw "bad"; }',
    'E_TYPE', /E_TYPE: throw requires an Error subclass/],
  ['recovery after error',
    'pub fnc main(): int { return 42; }',
    'main() -> 42', /^main\(\) -> 42$/],
];

let failed = false;
try {
  for (const name of names) {
    let browser;
    try {
      browser = await engines[name].launch({headless:true, timeout:60000});
      const page = await browser.newPage();
      const pageErrors = [];
      page.on('pageerror', err => pageErrors.push(String(err)));
      const navigation = await page.goto(url, {waitUntil:'load', timeout:30000});
      assert.equal(navigation?.status(), 200, 'test page HTTP status');
      await page.waitForFunction(() => window.__attoReady === true, null, {timeout:15000});
      const ua = await page.evaluate(() => navigator.userAgent);
      for (const [label, source, want, expected] of tests) {
        await page.locator('#source').fill(source);
        await page.locator('#run').click();
        try {
          await page.waitForFunction(
            needle => document.querySelector('#result')?.textContent.includes(needle),
            want, {timeout:20000});
        } catch (error) {
          const actual = await page.locator('#result').textContent();
          throw new Error(name + ' ' + label + ': ' + error.message + '; actual output=' + actual);
        }
        const output = await page.locator('#result').textContent();
        assert.match(output, expected, name + ' ' + label);
        console.log('PASS ' + name + ': ' + label);
      }
      assert.deepEqual(pageErrors, [], name + ' page errors');
      await page.close();
      console.log(JSON.stringify({engine:name, status:'PASS', userAgent:ua, cases:tests.length,
        actual_web_worker:true, source_git_blob:sourceBlob}));
    } catch (error) {
      failed = true;
      console.error('FAIL ' + name + ': ' + String(error));
    } finally {
      await browser?.close();
    }
  }
  assert.ok(requests.some(([m,p]) => m === 'GET' && p === '/atto_compiler.wasm'),
    'browser must fetch actual compiled Rust/Wasm compiler');
  assert.ok(requests.every(([method]) => method === 'GET'), 'source must not be POSTed to the host');
} finally {
  await new Promise(resolve => server.close(resolve));
}
if (failed) process.exitCode = 1;

// Dev-only stand-in for `openstance serve`, serving the fictional test
// fixtures so the UI can be viewed without a graph:
//   node scripts/mock-api.mjs            # 127.0.0.1:7499
//   OPENSTANCE_API=http://127.0.0.1:7499 npm run dev
//   open http://localhost:5173/#token=dev
// Not used by tests and not shipped in dist/.
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';

const PORT = Number(process.env.PORT ?? 7499);
const fx = (name) => JSON.parse(readFileSync(new URL(`../src/test/fixtures/${name}.json`, import.meta.url), 'utf8'));

const routes = {
  'GET /health': () => fx('health'),
  'GET /stats': () => fx('stats'),
  'GET /config': () => fx('config'),
  'POST /search': () => fx('search'),
  'GET /connections': () => fx('connections-812'),
  'GET /views': () => fx('views'),
  'GET /review': () => fx('review'),
  'GET /sources': () => fx('sources'),
  'GET /enrich-log': () => fx('enrich-log'),
  'GET /shortlist': () => fx('shortlist'),
  'GET /intros': () => fx('intros'),
  'POST /edits': () => ({ v: 1, applied: 1, people: [] }),
  'POST /import': () => fx('import'),
};

createServer((req, res) => {
  const url = new URL(req.url ?? '/', `http://127.0.0.1:${PORT}`);
  let body = '';
  req.on('data', (c) => (body += c));
  req.on('end', () => {
    const send = (status, json) => {
      res.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' });
      res.end(JSON.stringify(json));
    };
    if (url.pathname !== '/health' && !String(req.headers.authorization ?? '').startsWith('Bearer ')) {
      return send(401, { v: 1, error: { code: 'unauthorized', message: 'missing bearer token' } });
    }
    const key = `${req.method} ${url.pathname}`;
    if (/^GET \/people\/\d+$/.test(key)) return send(200, fx('person-812'));
    if (/^POST \/playbooks\//.test(key)) {
      let allow;
      try {
        allow = JSON.parse(body || '{}').allow_egress === true;
      } catch {
        allow = false;
      }
      return send(200, fx(allow ? 'outcome-path-done' : 'outcome-path-quote'));
    }
    const route = routes[key];
    if (route) return send(200, route());
    return send(404, { v: 1, error: { code: 'not_found', message: `mock: no route for ${key}` } });
  });
}).listen(PORT, '127.0.0.1', () => console.log(`mock openstance API on http://127.0.0.1:${PORT}`));

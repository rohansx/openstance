import { vi } from 'vitest';

// A fetch mock for the openstance HTTP API. Routes are keyed by
// "METHOD /path" (path without query string). A route value is either a JSON
// body (200) or a function returning { status, body } or a Response.
// Every request is recorded so tests can assert on payloads.

export interface RecordedCall {
  method: string;
  path: string;
  url: URL;
  headers: Headers;
  body: unknown;
}

export type Reply = { status?: number; body?: unknown; raw?: BodyInit; contentType?: string };
export type RouteHandler = (call: RecordedCall) => Reply | Promise<Reply>;
export type Routes = Record<string, unknown>;

function isReply(v: unknown): v is Reply {
  return typeof v === 'object' && v !== null && ('status' in v || 'raw' in v) && !('v' in v);
}

export function mockApi(routes: Routes = {}) {
  const calls: RecordedCall[] = [];
  const table: Routes = { ...routes };

  const fetchMock = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, 'http://127.0.0.1:7411');
    const method = (init.method ?? 'GET').toUpperCase();
    const headers = new Headers(init.headers);
    let body: unknown = init.body ?? null;
    if (typeof body === 'string') {
      try {
        body = JSON.parse(body);
      } catch {
        // keep the raw string
      }
    }
    const call: RecordedCall = { method, path: url.pathname, url, headers, body };
    calls.push(call);

    const key = `${method} ${url.pathname}`;
    let route = table[key];
    if (route === undefined) {
      return new Response(JSON.stringify({ v: 1, error: { code: 'not_found', message: `no mock for ${key}` } }), {
        status: 404,
        headers: { 'content-type': 'application/json' },
      });
    }
    if (typeof route === 'function') route = await (route as RouteHandler)(call);
    if (route instanceof Response) return route;
    const reply: Reply = isReply(route) ? route : { status: 200, body: route };
    if (reply.raw !== undefined) {
      return new Response(reply.raw, {
        status: reply.status ?? 200,
        headers: { 'content-type': reply.contentType ?? 'application/octet-stream' },
      });
    }
    return new Response(reply.body === undefined ? null : JSON.stringify(reply.body), {
      status: reply.status ?? 200,
      headers: { 'content-type': 'application/json; charset=utf-8' },
    });
  });

  vi.stubGlobal('fetch', fetchMock);

  return {
    calls,
    fetch: fetchMock,
    set(key: string, value: unknown) {
      table[key] = value;
    },
    callsTo(key: string): RecordedCall[] {
      const [method, path] = key.split(' ');
      return calls.filter((c) => c.method === method && c.path === path);
    },
  };
}

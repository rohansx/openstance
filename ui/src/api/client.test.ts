import { describe, expect, it, vi } from 'vitest';
import { fx } from '../test/fixtures';
import { mockApi } from '../test/mockApi';
import { ApiError, createClient } from './client';

const client = (opts: Partial<Parameters<typeof createClient>[0]> = {}) =>
  createClient({ base: '', getToken: () => 'tok123', ...opts });

describe('api client', () => {
  it('sends the bearer token on every route except /health', async () => {
    const api = mockApi({ 'GET /health': fx.health(), 'GET /people/812': fx.person() });
    const c = client();
    await c.health();
    await c.person(812);
    const [health, person] = api.calls;
    expect(health!.headers.get('authorization')).toBeNull();
    expect(person!.headers.get('authorization')).toBe('Bearer tok123');
  });

  it('never sends cookies', async () => {
    const api = mockApi({ 'GET /people/812': fx.person() });
    await client().person(812);
    const init = api.fetch.mock.calls[0]![1] as RequestInit;
    expect(init.credentials).toBe('omit');
  });

  it('prefixes the configured base (VITE_API_BASE)', async () => {
    const api = mockApi({ 'GET /people/812': fx.person() });
    await client({ base: 'http://127.0.0.1:7411' }).person(812);
    expect(String(api.fetch.mock.calls[0]![0])).toBe('http://127.0.0.1:7411/people/812?interactions=10&mutuals=10');
  });

  it('POSTs the Query JSON to /search', async () => {
    const api = mockApi({ 'POST /search': fx.search() });
    const res = await client().search({ text: 'devtools dinner', met_in_person: true, parse: false });
    expect(res.results[0]!.person.display_name).toBe('Tom Okafor');
    const call = api.callsTo('POST /search')[0]!;
    expect(call.headers.get('content-type')).toContain('application/json');
    expect(call.body).toEqual({ text: 'devtools dinner', met_in_person: true, parse: false });
  });

  it('turns the error envelope into an ApiError with code, status and candidates', async () => {
    mockApi({ 'GET /connections': { status: 409, body: fx.ambiguous() } });
    const err = await client()
      .connections('Sam')
      .catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    const e = err as ApiError;
    expect(e.status).toBe(409);
    expect(e.code).toBe('ambiguous');
    expect(e.message).toBe('3 people match "Sam"');
    expect(e.candidates.map((c) => c.id)).toEqual([3, 9]);
  });

  it('calls onUnauthorized on a 401', async () => {
    mockApi({ 'GET /views': { status: 401, body: fx.unauthorized() } });
    const onUnauthorized = vi.fn();
    await expect(client({ onUnauthorized }).views()).rejects.toMatchObject({ code: 'unauthorized' });
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });

  it('posts edit ops as { ops }', async () => {
    const api = mockApi({ 'POST /edits': { applied: 1, people: [] } });
    await client().edits([{ op: 'set', person: 812, field: 'title', value: 'CTO' }]);
    expect(api.callsTo('POST /edits')[0]!.body).toEqual({
      ops: [{ op: 'set', person: 812, field: 'title', value: 'CTO' }],
    });
  });

  it('runs a playbook with allow_egress false unless told otherwise', async () => {
    const api = mockApi({ 'POST /playbooks/path': fx.quoteOutcome() });
    const out = await client().playbook('path', { input: { target: 'Priya Raman', company: 'Stripe' } });
    expect(out.status).toBe('quote');
    expect(api.callsTo('POST /playbooks/path')[0]!.body).toEqual({
      input: { target: 'Priya Raman', company: 'Stripe' },
      allow_egress: false,
      max_usd: null,
      plan_only: false,
    });
  });

  it('encodes repeated query keys and path segments', async () => {
    const api = mockApi({ 'GET /enrich-log': fx.enrichLog(), 'GET /views/a%2Fb/search': { v: 1, results: [] } });
    await client().enrichLog({ run: '01JC', since: '30d' });
    await client().searchView('a/b', { q: 'x' });
    expect(api.calls[0]!.url.search).toBe('?run=01JC&since=30d');
    expect(api.calls[1]!.url.pathname).toBe('/views/a%2Fb/search');
  });

  it('downloads a view export as bytes', async () => {
    mockApi({ 'POST /views/devtools-investors/export': { raw: 'age-encryption.org/v1', contentType: 'application/octet-stream' } });
    const blob = await client().exportView('devtools-investors');
    expect(await blob.text()).toBe('age-encryption.org/v1');
  });

  it('normalizes the shortlist and intros lists', async () => {
    mockApi({ 'GET /shortlist': fx.shortlist(), 'GET /intros': fx.intros() });
    const c = client();
    expect((await c.shortlist())[0]!.person_id).toBe(812);
    expect((await c.intros())[0]!.stage).toBe('asked');
  });

  it('shortlist and intro writes use the documented payloads', async () => {
    const intro = fx.intros().intros[0]!;
    const entry = fx.shortlist().shortlist[0]!;
    const api = mockApi({
      'POST /shortlist': { v: 1, entry },
      'PATCH /shortlist/812': { v: 1, entry: { ...entry, note: 'n' } },
      'DELETE /shortlist/812': { v: 1, removed: true },
      'POST /intros': { v: 1, intro: { ...intro, stage: 'draft' } },
      'PATCH /intros/1': { v: 1, intro },
      'DELETE /intros/1': { v: 1, removed: true },
    });
    const c = client();
    expect((await c.addToShortlist(812)).person_id).toBe(812);
    expect((await c.setShortlistNote(812, 'n')).note).toBe('n');
    await c.removeFromShortlist(812);
    expect((await c.createIntro({ person: 1203, connector: 812, stage: 'draft' })).stage).toBe('draft');
    expect((await c.updateIntro(1, { stage: 'asked' })).stage).toBe('asked');
    await c.deleteIntro(1);
    expect(api.calls.map((x) => `${x.method} ${x.path}`)).toEqual([
      'POST /shortlist',
      'PATCH /shortlist/812',
      'DELETE /shortlist/812',
      'POST /intros',
      'PATCH /intros/1',
      'DELETE /intros/1',
    ]);
    expect(api.calls[0]!.body).toEqual({ person: 812, note: null });
    expect(api.calls[1]!.body).toEqual({ note: 'n' });
    expect(api.calls[3]!.body).toEqual({ person: 1203, connector: 812, stage: 'draft', note: null, follow_up: null });
    expect(api.calls[4]!.body).toEqual({ stage: 'asked' });
  });
});

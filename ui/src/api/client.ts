// Typed client for the openstance HTTP API (interfaces.md section 3).
//
// - `Authorization: Bearer <token>` on every route except GET /health.
// - No cookies (`credentials: 'omit'`).
// - Errors arrive as `{ v, error: { code, message, candidates? } }` and are
//   thrown as `ApiError`.
// - A playbook quote is not an error: it is a 200 with `status: "quote"`.

import type {
  ConfigJson,
  CreateViewRequest,
  EditOp,
  EditsResponse,
  EnrichLogResponse,
  ErrorBody,
  Health,
  HowConnected,
  ImportResponse,
  Intro,
  IntroPatch,
  IntroStage,
  NewIntro,
  Outcome,
  PersonDetail,
  PersonRefArg,
  PlaybookRequest,
  Quote,
  ReviewQueue,
  SearchRequest,
  SearchResponse,
  ShortlistEntry,
  SourceRow,
  Stats,
  TieDetail,
  ViewSearchResult,
  ViewSummary,
} from './types';

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly candidates: { id: number; display_name: string | null }[];

  constructor(status: number, code: string, message: string, candidates: ApiError['candidates'] = []) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
    this.candidates = candidates;
  }
}

export interface ClientOptions {
  /** '' for same-origin (embedded UI, Vite proxy) or e.g. http://127.0.0.1:7411. */
  base: string;
  getToken: () => string | null;
  onUnauthorized?: () => void;
}

type QueryValue = string | number | boolean | null | undefined | (string | number)[];
type Params = Record<string, QueryValue>;

export function toSearchParams(params: Params): string {
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '') continue;
    if (Array.isArray(v)) v.forEach((x) => sp.append(k, String(x)));
    else sp.append(k, String(v));
  }
  const s = sp.toString();
  return s ? `?${s}` : '';
}

const seg = (s: string | number) => encodeURIComponent(String(s));

export interface ImportMeta {
  kind?: string;
  label?: string;
  bodies?: boolean;
}

export interface UploadProgress {
  loaded: number;
  total: number;
}

export function createClient(opts: ClientOptions) {
  const base = opts.base.replace(/\/+$/, '');

  function authHeaders(): Record<string, string> {
    const t = opts.getToken();
    return t ? { authorization: `Bearer ${t}` } : {};
  }

  async function fail(res: Response): Promise<never> {
    let code = res.status === 401 ? 'unauthorized' : 'internal';
    let message = res.statusText || `HTTP ${res.status}`;
    let candidates: ApiError['candidates'] = [];
    try {
      const body = (await res.json()) as Partial<ErrorBody>;
      if (body?.error) {
        code = body.error.code ?? code;
        message = body.error.message ?? message;
        candidates = body.error.candidates ?? [];
      }
    } catch {
      // non-JSON error body (e.g. 421 from the Host check)
    }
    if (res.status === 421) message = `${message} (the server rejected this Host; open the UI at 127.0.0.1 or localhost)`;
    if (res.status === 401) opts.onUnauthorized?.();
    throw new ApiError(res.status, code, message, candidates);
  }

  async function raw(method: string, path: string, body?: unknown, auth = true): Promise<Response> {
    const headers: Record<string, string> = { accept: 'application/json', ...(auth ? authHeaders() : {}) };
    const init: RequestInit = { method, headers, credentials: 'omit', cache: 'no-store' };
    if (body instanceof FormData) {
      // the browser sets the multipart boundary
      init.body = body;
    } else if (body !== undefined) {
      headers['content-type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    let res: Response;
    try {
      res = await fetch(`${base}${path}`, init);
    } catch (e) {
      throw new ApiError(0, 'network', `Cannot reach openstance serve (${(e as Error).message}). Is it running?`);
    }
    if (!res.ok) await fail(res);
    return res;
  }

  async function json<T>(method: string, path: string, body?: unknown, auth = true): Promise<T> {
    const res = await raw(method, path, body, auth);
    if (res.status === 204) return undefined as T;
    const text = await res.text();
    return (text ? JSON.parse(text) : undefined) as T;
  }

  const pick = <T>(obj: unknown, key: string): T[] => {
    if (Array.isArray(obj)) return obj as T[];
    const v = (obj as Record<string, unknown> | null)?.[key];
    return Array.isArray(v) ? (v as T[]) : [];
  };

  return {
    health: () => json<Health>('GET', '/health', undefined, false),

    search: (q: SearchRequest) => json<SearchResponse>('POST', '/search', q),

    person: (id: number | string, p: { interactions?: number; mutuals?: number } = {}) =>
      json<PersonDetail>(
        'GET',
        `/people/${seg(id)}${toSearchParams({ interactions: p.interactions ?? 10, mutuals: p.mutuals ?? 10 })}`,
      ),

    ties: (id: number, other: number, limit = 10) =>
      json<TieDetail>('GET', `/people/${seg(id)}/ties/${seg(other)}${toSearchParams({ limit })}`),

    connections: (target: string | number, limit = 3) =>
      json<HowConnected>('GET', `/connections${toSearchParams({ target: String(target), limit })}`),

    playbook: (name: string, req: Partial<PlaybookRequest> & { input: Record<string, unknown> }) =>
      json<Outcome>('POST', `/playbooks/${seg(name)}`, {
        input: req.input,
        allow_egress: req.allow_egress === true,
        max_usd: req.max_usd ?? null,
        plan_only: req.plan_only === true,
      }),

    quote: (requests: Record<string, unknown>[]) => json<Quote>('POST', '/quote', { requests }),

    edits: (ops: EditOp[]) => json<EditsResponse>('POST', '/edits', { ops }),

    review: (minConfidence?: number) =>
      json<ReviewQueue>('GET', `/review${toSearchParams({ min_confidence: minConfidence })}`),

    views: (direction?: 'own' | 'imported' | 'all') =>
      json<{ views: ViewSummary[] }>('GET', `/views${toSearchParams({ direction })}`).then((r) =>
        pick<ViewSummary>(r, 'views'),
      ),

    createView: (req: CreateViewRequest) => json<ViewSummary>('POST', '/views', req),

    searchView: (name: string, p: { q?: string; company?: string[]; title?: string[]; limit?: number } = {}) =>
      json<{ results: ViewSearchResult[] }>('GET', `/views/${seg(name)}/search${toSearchParams(p)}`),

    exportView: async (name: string, armor = false): Promise<Blob> => {
      const res = await raw('POST', `/views/${seg(name)}/export${toSearchParams({ armor: armor || undefined })}`);
      return res.blob();
    },

    importView: (file: File) => {
      const form = new FormData();
      form.append('file', file, file.name);
      return json<ViewSummary>('POST', '/views/import', form);
    },

    sources: () => json<{ sources: SourceRow[] }>('GET', '/sources').then((r) => pick<SourceRow>(r, 'sources')),

    enrichLog: (p: { run?: string; since?: string; person?: string | number } = {}) =>
      json<EnrichLogResponse>('GET', `/enrich-log${toSearchParams(p)}`),

    config: () => json<ConfigJson>('GET', '/config'),

    stats: () => json<Stats>('GET', '/stats'),

    shortlist: () => json<{ shortlist: ShortlistEntry[] }>('GET', '/shortlist').then((r) => pick<ShortlistEntry>(r, 'shortlist')),
    addToShortlist: (person: PersonRefArg, note: string | null = null) =>
      json<{ entry: ShortlistEntry }>('POST', '/shortlist', { person, note }).then((r) => r.entry),
    setShortlistNote: (person: number, note: string | null) =>
      json<{ entry: ShortlistEntry }>('PATCH', `/shortlist/${seg(person)}`, { note }).then((r) => r.entry),
    removeFromShortlist: (person: number) => json<{ removed: boolean }>('DELETE', `/shortlist/${seg(person)}`),

    intros: (stage?: IntroStage) =>
      json<{ intros: Intro[] }>('GET', `/intros${toSearchParams({ stage })}`).then((r) => pick<Intro>(r, 'intros')),
    createIntro: (intro: NewIntro) =>
      json<{ intro: Intro }>('POST', '/intros', {
        person: intro.person,
        connector: intro.connector ?? null,
        stage: intro.stage,
        note: intro.note ?? null,
        follow_up: intro.follow_up ?? null,
      }).then((r) => r.intro),
    updateIntro: (id: number, patch: IntroPatch) =>
      json<{ intro: Intro }>('PATCH', `/intros/${seg(id)}`, patch).then((r) => r.intro),
    deleteIntro: (id: number) => json<{ removed: boolean }>('DELETE', `/intros/${seg(id)}`),

    /**
     * POST /import (multipart, repeatable `file`). Uses XMLHttpRequest so the
     * page can show upload progress; fetch cannot report it.
     */
    importFiles: (files: File[], meta: ImportMeta = {}, onProgress?: (p: UploadProgress) => void) =>
      new Promise<ImportResponse>((resolve, reject) => {
        const form = new FormData();
        files.forEach((f) => form.append('file', f, f.name));
        if (meta.kind) form.append('kind', meta.kind);
        if (meta.label) form.append('label', meta.label);
        if (meta.bodies) form.append('bodies', 'true');

        const xhr = new XMLHttpRequest();
        xhr.open('POST', `${base}/import`);
        xhr.withCredentials = false;
        const t = opts.getToken();
        if (t) xhr.setRequestHeader('authorization', `Bearer ${t}`);
        xhr.setRequestHeader('accept', 'application/json');
        xhr.upload.onprogress = (e) => {
          if (onProgress) onProgress({ loaded: e.loaded, total: e.lengthComputable ? e.total : 0 });
        };
        xhr.onerror = () => reject(new ApiError(0, 'network', 'Upload failed: cannot reach openstance serve.'));
        xhr.onload = () => {
          let body: unknown;
          try {
            body = xhr.responseText ? JSON.parse(xhr.responseText) : null;
          } catch {
            body = null;
          }
          if (xhr.status >= 200 && xhr.status < 300) {
            resolve(body as ImportResponse);
            return;
          }
          const err = (body as Partial<ErrorBody> | null)?.error;
          if (xhr.status === 401) opts.onUnauthorized?.();
          reject(new ApiError(xhr.status, err?.code ?? 'internal', err?.message ?? `HTTP ${xhr.status}`));
        };
        xhr.send(form);
      }),
  };
}

export type Client = ReturnType<typeof createClient>;

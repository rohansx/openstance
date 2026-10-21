import type {
  ConfigJson,
  EnrichLogResponse,
  ErrorBody,
  Health,
  HowConnected,
  ImportResponse,
  Intro,
  Outcome,
  PersonDetail,
  ReviewQueue,
  SearchResponse,
  ShortlistEntry,
  SourceRow,
  Stats,
  ViewSummary,
} from '../api/types';
import ambiguousJson from './fixtures/ambiguous.json';
import configJson from './fixtures/config.json';
import connectionsJson from './fixtures/connections-812.json';
import enrichLogJson from './fixtures/enrich-log.json';
import healthJson from './fixtures/health.json';
import importJson from './fixtures/import.json';
import introsJson from './fixtures/intros.json';
import doneJson from './fixtures/outcome-path-done.json';
import quoteJson from './fixtures/outcome-path-quote.json';
import personJson from './fixtures/person-812.json';
import reviewJson from './fixtures/review.json';
import searchJson from './fixtures/search.json';
import shortlistJson from './fixtures/shortlist.json';
import sourcesJson from './fixtures/sources.json';
import statsJson from './fixtures/stats.json';
import unauthorizedJson from './fixtures/unauthorized.json';
import viewsJson from './fixtures/views.json';

const clone = <T>(x: unknown): T => structuredClone(x) as T;

export const fx = {
  search: () => clone<SearchResponse>(searchJson),
  person: () => clone<PersonDetail>(personJson),
  connections: () => clone<HowConnected>(connectionsJson),
  quoteOutcome: () => clone<Outcome>(quoteJson),
  doneOutcome: () => clone<Outcome>(doneJson),
  views: () => clone<{ v: number; views: ViewSummary[] }>(viewsJson),
  review: () => clone<ReviewQueue>(reviewJson),
  importStats: () => clone<ImportResponse>(importJson),
  enrichLog: () => clone<EnrichLogResponse>(enrichLogJson),
  unauthorized: () => clone<ErrorBody>(unauthorizedJson),
  ambiguous: () => clone<ErrorBody>(ambiguousJson),
  health: () => clone<Health>(healthJson),
  sources: () => clone<{ v: number; sources: SourceRow[] }>(sourcesJson),
  stats: () => clone<Stats & { v: number }>(statsJson),
  shortlist: () => clone<{ v: number; shortlist: ShortlistEntry[] }>(shortlistJson),
  intros: () => clone<{ v: number; intros: Intro[] }>(introsJson),
  config: () => clone<ConfigJson>(configJson),
};

/** Default routes for rendering the whole app. */
export function defaultRoutes(): Record<string, unknown> {
  return {
    'GET /health': fx.health(),
    'GET /stats': fx.stats(),
    'GET /config': fx.config(),
    'POST /search': fx.search(),
    'GET /people/812': fx.person(),
    'GET /connections': fx.connections(),
    'GET /views': fx.views(),
    'GET /review': fx.review(),
    'GET /sources': fx.sources(),
    'GET /enrich-log': fx.enrichLog(),
    'GET /shortlist': fx.shortlist(),
    'GET /intros': fx.intros(),
  };
}

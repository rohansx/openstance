// JSON shapes of the openstance HTTP API.
//
// Source of truth: docs/tech-spec/interfaces.md section 1 (shared shapes) and
// section 3 (endpoints), checked against the Rust serializers in
// crates/openstance-core/src/json.rs. Every top-level response carries
// `v: 1`. Money is integer micro-USD. Timestamps are ISO 8601 UTC strings.
// `null` means unknown; arrays are always present.
//
// The shortlist, intros and stats endpoints are not in interfaces.md yet; their
// shapes here are the UI's assumption (see ui/README.md) and are reconciled
// with the API stream at integration.

export type Iso = string;
export type MicroUsd = number;

// 1.1
export interface PersonRef {
  id: number;
  display_name: string | null;
  title?: string | null;
  company?: string | null;
  score?: number;
}

// 1.3
export interface Weights {
  recency: number;
  frequency: number;
  reciprocity: number;
  meetings: number;
  breadth: number;
}

export interface ScoreRaw {
  days_since_last: number | null;
  n_interactions: number;
  messages_from_you: number;
  replies_from_them: number;
  threads_they_started: number;
  n_meetings: number;
  channels: string[];
}

export interface ScoreComponents extends Weights {
  weights: Weights;
  preset: string;
  half_life_days: number;
  raw: ScoreRaw | null;
}

export type ScalarField = 'display_name' | 'title' | 'company' | 'location' | 'bio';

// 1.2
export interface Person {
  id: number;
  display_name: string | null;
  title: string | null;
  company: string | null;
  location: string | null;
  bio: string | null;
  tags: string[];
  first_seen: Iso | null;
  last_contact: Iso | null;
  score: number;
  components: ScoreComponents;
  is_bulk: boolean;
  do_not_share: boolean;
  no_enrich: boolean;
  /** `override`, `<source.kind>:<source.label>`, or null. */
  field_sources: Record<string, string | null>;
}

export interface SourceRef {
  id: number;
  kind: string;
  label: string | null;
}

export interface Verification {
  status: 'deliverable' | 'undeliverable' | 'risky' | 'unknown' | string;
  at: Iso;
  via: string;
}

export type IdentityKind =
  | 'email'
  | 'phone'
  | 'linkedin_url'
  | 'x_handle'
  | 'x_user_id'
  | 'ig_handle'
  | 'name_company'
  | 'name_local';

// 1.4
export interface Identity {
  id: number;
  kind: IdentityKind | string;
  value: string;
  confidence: number | null;
  match_rule?: string | null;
  source: SourceRef;
  verification?: Verification;
}

// 1.5
export interface InteractionEvidence {
  kind: 'interaction';
  interaction_id: number;
  type: 'email' | 'meeting' | 'dm' | 'mention' | 'follow' | 'connection' | string;
  direction: 'in' | 'out' | 'mutual' | 'none' | null;
  at: Iso;
  subject: string | null;
  role: string | null;
  source: SourceRef;
  /** Rendered line; present in the Rust output, absent in some spec examples. */
  line?: string;
}

export interface FieldEvidence {
  kind: 'field';
  field: string;
  value: string;
  matched: string;
}

export interface TieEvidence {
  kind: 'tie';
  with: { id: number; display_name: string | null };
  tie_kind: string;
  n_shared: number;
  weight: number;
  last_at: Iso;
}

export interface EnrichEvidence {
  kind: 'enrich';
  op: string;
  field: string;
  value: string;
  backend: string;
  at: Iso;
  enrich_log_id: number;
}

export type Evidence = InteractionEvidence | FieldEvidence | TieEvidence | EnrichEvidence;

// 1.6 — the parsed `Query`. `since`/`until`/`last_contact_before` are ISO
// strings in the spec; the Rust struct serializes unix seconds. Both are
// accepted on read; the UI always sends strings (the MCP input format).
export type DateValue = string | number | null;

export interface Query {
  text: string | null;
  company: string[];
  title: string[];
  location: string[];
  industry: string[];
  tags: string[];
  since: DateValue;
  until: DateValue;
  last_contact_before: DateValue;
  met_in_person: boolean | null;
  replied: boolean | null;
  min_score: number | null;
  lost_touch: boolean | null;
  include_bulk: boolean;
  limit: number;
}

/** Body of `POST /search`: the MCP `search_people` input. */
export interface SearchRequest {
  text?: string;
  company?: string[];
  title?: string[];
  location?: string[];
  industry?: string[];
  tags?: string[];
  since?: string;
  until?: string;
  last_contact_before?: string;
  met_in_person?: boolean;
  replied?: boolean;
  min_score?: number;
  lost_touch?: boolean;
  include_bulk?: boolean;
  limit?: number;
  parse?: boolean;
  explain?: boolean;
}

export interface Relevance {
  fts: number;
  vector: number | null;
  fused: number;
  final: number;
}

export interface Matched {
  field: string;
  value?: unknown;
  matched?: string;
  via?: string;
  snippet?: string;
  similarity?: number;
}

export interface SearchResult {
  rank: number;
  person: Person;
  relevance: Relevance | null;
  matched: Matched[];
  evidence: Evidence[];
  reason: string | null;
  reason_source: string | null;
}

export interface SearchResponse {
  v: number;
  query: Query;
  query_source: { parser: string; fallback: string | null; explicit: string[] };
  rerun: string;
  mode: 'text' | 'structured';
  models: { parser: string; embedder: string; reranker: string };
  total: number;
  took_ms: number;
  results: SearchResult[];
  warnings: string[];
  suggestions: string[];
  timings_ms: Record<string, number> | null;
}

// 1.7
export interface PathJson {
  via: PersonRef;
  strength: number;
  reason: 'tie' | 'same_company' | 'past_company' | 'school' | string;
  my_score: number;
  link_weight: number;
  evidence: Evidence[];
}

export interface HowConnected {
  v: number;
  target: PersonRef;
  direct: { score: number; last_contact: Iso | null } | null;
  paths: PathJson[];
}

// 1.8
export interface Mutual {
  person: PersonRef;
  tie_kind: string;
  n_shared: number;
  weight: number;
}

export interface PersonDetail {
  v: number;
  resolved_from?: number;
  person: Person;
  identities: Identity[];
  recent_interactions: Evidence[];
  mutuals: Mutual[];
  overrides: { field: string; value: string | null; set_at: Iso }[];
  notes: { value: string; set_at: Iso }[];
  suggested: Record<string, { value: string; from: string; at: Iso }>;
  merge: { merged_from: number[]; flagged: boolean; confidence: number };
}

export interface TieDetail {
  tie: Record<string, unknown>;
  interactions: Evidence[];
}

// 1.9
export interface EnrichRequest {
  kind: 'profile' | 'find_email' | 'verify_email' | 'company' | 'web_search' | string;
  [field: string]: unknown;
}

export interface QuoteLine {
  job: { person_id: number | null; request: EnrichRequest };
  micro_usd: MicroUsd;
  basis: 'exact' | 'estimate' | 'free' | string;
  provider_hint: string | null;
}

export interface Skipped {
  person_id: number | null;
  op: string;
  reason: string;
}

export interface Quote {
  backend: string;
  lines: QuoteLine[];
  total_micro_usd: MicroUsd;
  budget_micro_usd: MicroUsd;
  within_budget: boolean;
  skipped: Skipped[];
  quoted_at: Iso | number;
}

export interface PlanStep {
  n: number;
  description: string;
  egress: string | null;
  quoted_micro_usd?: MicroUsd;
  will_run: string;
}

export interface Plan {
  playbook?: string;
  run_id: string;
  steps: PlanStep[];
  quote: Quote;
  egress_max: { op: string; max: number }[];
}

export interface Receipt {
  run_id: string;
  backend: string;
  requests_sent: number;
  quoted_micro_usd: MicroUsd;
  spent_micro_usd: MicroUsd;
  budget_micro_usd: MicroUsd;
  skipped: Skipped[];
  enrich_log_ids: number[];
}

export type RunStatus = 'done' | 'partial' | 'quote' | 'nothing_to_do';

export interface Outcome {
  v: number;
  playbook: string;
  status: RunStatus;
  plan: Plan;
  result: unknown;
  receipt: Receipt;
  notes: string[];
}

export interface PlaybookRequest {
  input: Record<string, unknown>;
  allow_egress: boolean;
  max_usd: number | null;
  plan_only: boolean;
}

// 1.10
export interface ViewSummary {
  id: string | null;
  name: string;
  direction: 'own' | 'imported' | string;
  owner: { pubkey: string | null; label: string | null };
  created_at: Iso;
  expires_at: Iso | null;
  rows: number;
  query: Partial<Query> | Record<string, unknown>;
  recipients?: string[];
  excluded_do_not_share?: number;
  exported_at?: Iso;
}

export interface CreateViewRequest {
  name: string;
  query: SearchRequest;
  ttl: string;
  to: string[];
}

// 1.11
export type ErrorCode =
  | 'bad_request'
  | 'unauthorized'
  | 'not_found'
  | 'ambiguous'
  | 'view_expired'
  | 'budget_exceeded'
  | 'egress_disabled'
  | 'db_busy'
  | 'schema_too_new'
  | 'internal';

export interface ErrorBody {
  v: number;
  error: {
    code: ErrorCode | string;
    message: string;
    candidates?: { id: number; display_name: string | null }[];
  };
}

// 3.2 Edit ops
export type EditOp =
  | { op: 'set'; person: number; field: ScalarField; value: string }
  | { op: 'unset'; person: number; field: ScalarField }
  | { op: 'tag'; person: number; value: string }
  | { op: 'untag'; person: number; value: string }
  | { op: 'note'; person: number; value: string }
  | { op: 'flag'; person: number; field: 'do_not_share' | 'no_enrich'; value: boolean }
  | { op: 'merge'; a: number; b: number }
  | { op: 'split'; person: number; identities: number[] }
  | { op: 'unmerge'; a: number; b: number };

export interface EditsResponse {
  applied: number;
  people: PersonRef[];
}

// Review queue (`GET /review`, core ReviewQueue)
export interface FlaggedMerge {
  person: PersonRef;
  rule: string;
  confidence: number;
  flagged_identities: Identity[];
  other_identities: Identity[];
  why: string;
}

export interface Suggestion {
  a: PersonRef;
  b: PersonRef;
  rule: string;
  confidence: number;
  why: string;
}

export interface ReviewQueue {
  v: number;
  flagged: FlaggedMerge[];
  suggestions: Suggestion[];
  info: { kind: string; message: string }[];
}

// Import (`POST /import`)
export interface SourceImport {
  id: number;
  kind: string;
  label: string | null;
  path?: string;
  identities: number;
  interactions: number;
  skipped_duplicate_file: boolean;
  stats?: Record<string, unknown>;
}

export interface ImportResponse {
  v: number;
  sources: SourceImport[];
  resolution: {
    people?: number;
    people_created: number;
    auto_merged: number;
    flagged: number;
    bulk?: number;
  };
  took_ms: number;
}

export interface SourceRow {
  id: number;
  kind: string;
  label: string | null;
  path?: string | null;
  imported_at?: Iso | null;
  [extra: string]: unknown;
}

// Enrich log (`GET /enrich-log`)
export interface EnrichLogRow {
  id: number;
  run_id: string;
  at: Iso;
  backend: string;
  provider: string | null;
  playbook: string | null;
  person_id: number | null;
  person_name: string | null;
  op: string;
  request: Record<string, unknown>;
  quoted_micro_usd: MicroUsd;
  cost_micro_usd: MicroUsd | null;
  status: 'quoted' | 'sent' | 'ok' | 'not_found' | 'error' | 'blocked' | string;
  error: string | null;
}

export interface EnrichLogResponse {
  v?: number;
  rows: EnrichLogRow[];
  /** interfaces.md 3.1 */
  total_micro_usd?: MicroUsd;
  /** core EnrichLogResponse */
  totals?: { requests_sent: number; spent_micro_usd: MicroUsd; quoted_only: number; blocked: number };
}

export interface Health {
  ok: boolean;
  version: string;
  schema_version: number;
}

// Assumed (not yet in interfaces.md): shortlist, intros, stats.
export interface ShortlistEntry {
  person_id: number;
  note: string | null;
  added_at?: Iso;
  person?: PersonRef | null;
}

export const INTRO_STAGES = ['shortlisted', 'asked', 'introduced', 'meeting'] as const;
export type IntroStage = (typeof INTRO_STAGES)[number];

export interface Intro {
  id: number;
  person_id: number;
  connector_id: number | null;
  stage: IntroStage | string;
  note: string | null;
  follow_up: string | null;
  created_at?: Iso;
  updated_at?: Iso;
  person?: PersonRef | null;
  connector?: PersonRef | null;
}

export interface NewIntro {
  person_id: number;
  connector_id?: number | null;
  stage: IntroStage;
  note?: string | null;
  follow_up?: string | null;
}

export type IntroPatch = Partial<Omit<NewIntro, 'person_id'>>;

export interface Stats {
  people: number;
  sources: number;
  last_import: Iso | null;
  flagged: number;
  enrich_total_micro_usd: MicroUsd;
}

export type ConfigJson = Record<string, unknown>;

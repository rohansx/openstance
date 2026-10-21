// JSON shapes of the openstance HTTP API.
//
// Source of truth: docs/tech-spec/interfaces.md section 1 (shared shapes) and
// section 3 (endpoints), checked against the Rust serializers in
// crates/openstance-core/src/json.rs. Every top-level response carries
// `v: 1`. Money is integer micro-USD. Timestamps are ISO 8601 UTC strings.
// `null` means unknown; arrays are always present.
//

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
  v: number;
  person: PersonRef;
  other: PersonRef;
  /** The strongest kind, or null. */
  tie: Tie | null;
  ties: Tie[];
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

export interface ViewPerson {
  display_name: string | null;
  title: string | null;
  company: string | null;
  location: string | null;
  score_bucket: 'strong' | 'medium' | 'weak' | string;
  last_contact_month: string | null;
  emails: string[];
  linkedin: string | null;
}

export interface ViewSearchResult {
  view: { id: string | null; name: string; owner: { label: string | null } };
  /** Present when `view = "*"` merged rows from several views. */
  views?: { id: string | null; name: string; owner: { label: string | null } }[];
  person: ViewPerson;
  relevance: number;
}

export interface CreateViewRequest {
  name: string;
  query: string | SearchRequest;
  ttl: string;
  /** age public keys or aliases (`self`). */
  to: string[];
  replace?: boolean;
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
// `person`, `a`, `b` take a person id or any reference string. A batch holds
// 1 to 500 ops and applies all or nothing.
export type EditOp =
  | { op: 'set'; person: PersonRefArg; field: ScalarField; value: string }
  | { op: 'unset'; person: PersonRefArg; field: ScalarField }
  | { op: 'tag'; person: PersonRefArg; value: string }
  | { op: 'untag'; person: PersonRefArg; value: string }
  | { op: 'note'; person: PersonRefArg; value: string }
  | { op: 'flag'; person: PersonRefArg; field: 'do_not_share' | 'no_enrich' | 'is_bulk'; value: boolean }
  | { op: 'merge'; a: PersonRefArg; b: PersonRefArg }
  | { op: 'split'; person: PersonRefArg; identities: number[] }
  | { op: 'unmerge'; a: PersonRefArg; b: PersonRefArg }
  /** accept a flagged merge (`review --confirm`) */
  | { op: 'confirm'; person: PersonRefArg }
  /** keep a suggested pair apart (`review --reject`) */
  | { op: 'reject'; a: PersonRefArg; b: PersonRefArg };

export interface EditsResponse {
  v?: number;
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
/** core `ImportStats` (interfaces.md 1.10), stored on each source row. */
export interface ImportStats {
  records_read: number;
  interactions_new: number;
  interactions_dup: number;
  identities_new: number;
  identities_seen: number;
  /** reason -> count */
  skipped: Record<string, number>;
  bytes: number;
  elapsed_ms: number;
  self_candidates: string[];
}

export interface SourceImport {
  id: number;
  kind: string;
  label: string | null;
  path?: string;
  identities: number;
  interactions: number;
  skipped_duplicate_file: boolean;
  stats?: ImportStats;
  /** handles (`kind:value`) this import adopted as "me" while `[me] identities` was empty */
  self_adopted?: string[];
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
  path: string | null;
  imported_at: Iso;
  importer_version?: number | null;
  /** null for enrichment sources. */
  stats?: ImportStats | null;
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
  v: number;
  /** Oldest first. */
  rows: EnrichLogRow[];
  totals: { requests_sent: number; spent_micro_usd: MicroUsd; quoted_only: number; blocked: number };
}

export interface Health {
  ok: boolean;
  version: string;
  schema_version: number;
}

// Workspace, dashboard and source shapes (interfaces.md section 3.4).
export interface ShortlistEntry {
  person_id: number;
  /** null only when the person no longer exists. */
  person: PersonRef | null;
  note: string | null;
  added_at: Iso;
}

export const INTRO_STAGES = ['draft', 'asked', 'intro_made', 'met', 'closed'] as const;
export type IntroStage = (typeof INTRO_STAGES)[number];

export interface Intro {
  id: number;
  person_id: number;
  person: PersonRef | null;
  /** null: a direct ask. */
  connector: PersonRef | null;
  stage: IntroStage;
  note: string | null;
  /** YYYY-MM-DD */
  follow_up: string | null;
  created_at: Iso;
  updated_at: Iso;
}

/** `<ref>`: a person id or any person reference string. */
export type PersonRefArg = number | string;

export interface NewIntro {
  person: PersonRefArg;
  connector?: PersonRefArg | null;
  stage: IntroStage;
  note?: string | null;
  follow_up?: string | null;
}

export interface IntroPatch {
  stage?: IntroStage;
  connector?: PersonRefArg | null;
  note?: string | null;
  follow_up?: string | null;
}

export interface Stats {
  v?: number;
  /** Excludes you and bulk senders. */
  people: number;
  bulk: number;
  sources: number;
  last_import: Iso | null;
  review: { flagged: number; suggestions: number };
  enrich: { requests_sent: number; spent_micro_usd: MicroUsd };
  shortlist: number;
  intros: Partial<Record<IntroStage, number>>;
  schema_version: number;
}

export interface Tie {
  tie_kind: string;
  n_shared: number;
  weight: number;
  last_at: Iso;
}

/** `GET /config`, same as `openstance config --json`. Secrets are never included. */
export interface ConfigJson {
  v?: number;
  config?: Record<string, unknown>;
  origins?: Record<string, string>;
  warnings?: string[];
  config_file?: string | null;
  file_found?: boolean;
  /** Resolved database file (absolute). Not exposed by every server build; see `dbPath`. */
  db_path?: string;
  [extra: string]: unknown;
}

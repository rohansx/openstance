//! Typed configuration loaded from `~/.openstance/config.toml`, environment
//! variables and built-in defaults. Spec: `docs/tech-spec/configuration.md`.
//!
//! Every key has a default, so a missing file and an empty file behave the
//! same. Unknown keys are warnings (errors with `strict`). Secrets are never
//! read from the file: keys that need one take an environment variable name.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The reference file (every key at its default, with comments).
pub const REFERENCE_TOML: &str = include_str!("../data/config.reference.toml");

/// Where the value of a key came from (`openstance config` annotates each key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Built-in default.
    Default,
    /// `config.toml`.
    File,
    /// `OPENSTANCE__SECTION__KEY` or a dedicated variable.
    Env,
    /// A CLI flag.
    Flag,
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// `[db]`.
    pub db: DbSection,
    /// `[score]`.
    pub score: ScoreSection,
    /// `[resolution]`.
    pub resolution: ResolutionSection,
    /// `[search]`.
    pub search: SearchSection,
    /// `[model]`.
    pub model: ModelSection,
    /// `[enrich]`.
    pub enrich: EnrichSection,
    /// `[me]`.
    pub me: MeSection,
    /// `[import]`.
    pub import: ImportSection,
    /// `[share]`.
    pub share: ShareSection,
    /// `[serve]`.
    pub serve: ServeSection,
    /// `[mcp]`.
    pub mcp: McpSection,
    /// `[playbooks.*]`.
    pub playbooks: PlaybooksSection,
    /// `[log]`.
    pub log: LogSection,
    /// `[sync]` (Phase 5).
    pub sync: SyncSection,
}

/// `[db]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DbSection {
    /// Database path (`~` and `$OPENSTANCE_HOME` expand; relative = under home).
    pub path: String,
    /// SQLite busy timeout, 0..=60000.
    pub busy_timeout_ms: u32,
}

impl Default for DbSection {
    fn default() -> Self {
        DbSection {
            path: "~/.openstance/graph.db".into(),
            busy_timeout_ms: 5000,
        }
    }
}

/// Explicit score weights (`[score] weights = {...}`). All five keys required.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeightsInput {
    /// R.
    pub recency: f64,
    /// F.
    pub frequency: f64,
    /// C.
    pub reciprocity: f64,
    /// M.
    pub meetings: f64,
    /// B.
    pub breadth: f64,
}

/// `[score]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoreSection {
    /// `default | founder | recruiter | seller | community`; `None` = `default`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Explicit weights; replaces the preset. Mutually exclusive with `preset`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weights: Option<WeightsInput>,
    /// R = 0.5 at this many days.
    pub half_life_days: f64,
    /// F saturation.
    pub frequency_scale: f64,
    /// M saturation.
    pub meetings_scale: f64,
    /// C: a thread they start counts this many times.
    pub thread_start_multiplier: f64,
    /// Tie weight saturation.
    pub tie_scale: f64,
    /// Interactions with more participants create no ties.
    pub tie_max_participants: u32,
    /// `[score.bulk]`.
    pub bulk: BulkSection,
}

impl Default for ScoreSection {
    fn default() -> Self {
        ScoreSection {
            preset: None,
            weights: None,
            half_life_days: 180.0,
            frequency_scale: 20.0,
            meetings_scale: 3.0,
            thread_start_multiplier: 2.0,
            tie_scale: 3.0,
            tie_max_participants: 20,
            bulk: BulkSection::default(),
        }
    }
}

impl ScoreSection {
    /// The preset name in effect: the configured one, `"custom"` for explicit
    /// weights, else `"default"`.
    pub fn effective_preset(&self) -> &str {
        match (&self.preset, &self.weights) {
            (Some(p), _) => p,
            (None, Some(_)) => "custom",
            (None, None) => "default",
        }
    }
}

/// `[score.bulk]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BulkSection {
    /// 50+ messages from them and zero replies either way -> bulk.
    pub min_messages: u32,
    /// Or List-Unsubscribe on more than this share of their messages.
    pub list_unsubscribe_ratio: f64,
}

impl Default for BulkSection {
    fn default() -> Self {
        BulkSection {
            min_messages: 50,
            list_unsubscribe_ratio: 0.5,
        }
    }
}

/// `[resolution]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolutionSection {
    /// Jaro-Winkler suggestion threshold (never auto-merges).
    pub suggest_jaro_winkler: f64,
    /// Size of the bundled common-names lists.
    pub common_names_top_n: u32,
    /// Common name at a company this big -> suggestion, not merge.
    pub large_company_threshold: u32,
    /// Company-name normalization additions, `alias -> canonical`.
    pub company_aliases: BTreeMap<String, String>,
    /// Extra freemail domains.
    pub freemail_extra: Vec<String>,
    /// Extra role local parts.
    pub role_localparts_extra: Vec<String>,
    /// Domains where dots and `+tag` are ignored (rule 2). The first entry is
    /// the canonical domain of the alias key. Tests set `gmail.test`.
    pub gmail_domains: Vec<String>,
}

impl Default for ResolutionSection {
    fn default() -> Self {
        ResolutionSection {
            suggest_jaro_winkler: 0.92,
            common_names_top_n: 5000,
            large_company_threshold: 50,
            company_aliases: BTreeMap::new(),
            freemail_extra: Vec::new(),
            role_localparts_extra: Vec::new(),
            gmail_domains: vec!["gmail.com".into(), "googlemail.com".into()],
        }
    }
}

/// `[search]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSection {
    /// CLI default `--limit`.
    pub default_limit: u32,
    /// Top-N FTS5 hits.
    pub candidates_fts: u32,
    /// Top-N vector hits.
    pub candidates_vec: u32,
    /// FTS share of the fused score.
    pub text_weight: f64,
    /// `final = fused * ((1 - lift) + lift * score)`.
    pub relationship_lift: f64,
    /// Evidence lines per result.
    pub evidence_per_result: u32,
    /// Include bulk senders by default.
    pub include_bulk: bool,
}

impl Default for SearchSection {
    fn default() -> Self {
        SearchSection {
            default_limit: 30,
            candidates_fts: 200,
            candidates_vec: 200,
            text_weight: 0.5,
            relationship_lift: 0.5,
            evidence_per_result: 3,
            include_bulk: false,
        }
    }
}

/// `[model]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelSection {
    /// Embedder id.
    pub embedder: String,
    /// Parser id.
    pub parser: String,
    /// Reranker id.
    pub reranker: String,
    /// Model cache directory.
    pub cache_dir: String,
    /// Per call to a local model.
    pub timeout_local_ms: u32,
    /// Per call to a remote model.
    pub timeout_remote_ms: u32,
    /// Candidates sent to the reranker.
    pub rerank_top_n: u32,
    /// `[model.ollama]`.
    pub ollama: OllamaSection,
    /// `[model.openai]`.
    pub openai: OpenAiSection,
    /// `[model.anthropic]`.
    pub anthropic: AnthropicSection,
}

impl Default for ModelSection {
    fn default() -> Self {
        ModelSection {
            embedder: "fastembed:bge-small-en-v1.5".into(),
            parser: "rules".into(),
            reranker: "none".into(),
            cache_dir: "~/.openstance/models".into(),
            timeout_local_ms: 5000,
            timeout_remote_ms: 10000,
            rerank_top_n: 50,
            ollama: OllamaSection::default(),
            openai: OpenAiSection::default(),
            anthropic: AnthropicSection::default(),
        }
    }
}

/// `[model.ollama]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaSection {
    /// Server URL.
    pub base_url: String,
}

impl Default for OllamaSection {
    fn default() -> Self {
        OllamaSection {
            base_url: "http://127.0.0.1:11434".into(),
        }
    }
}

/// `[model.openai]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiSection {
    /// API base URL.
    pub base_url: String,
    /// Env var holding the key.
    pub api_key_env: String,
    /// `schema | object | prompt`.
    pub json_mode: String,
    /// Embedding dimensions (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<u32>,
}

impl Default for OpenAiSection {
    fn default() -> Self {
        OpenAiSection {
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            json_mode: "schema".into(),
            dimensions: None,
        }
    }
}

/// `[model.anthropic]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnthropicSection {
    /// Env var holding the key.
    pub api_key_env: String,
}

impl Default for AnthropicSection {
    fn default() -> Self {
        AnthropicSection {
            api_key_env: "ANTHROPIC_API_KEY".into(),
        }
    }
}

/// `[enrich]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnrichSection {
    /// `none | treg | ownkeys`.
    pub backend: String,
    /// Per playbook run budget.
    pub max_usd_per_run: f64,
    /// `allow_egress` when an MCP/HTTP caller omits it.
    pub allow_egress_default: bool,
    /// Also skip `do_not_share` people in enrichment.
    pub respect_do_not_share: bool,
    /// Per request timeout.
    pub timeout_secs: u32,
    /// `[enrich.treg]`.
    pub treg: TregSection,
    /// `[enrich.ownkeys]`.
    pub ownkeys: OwnKeysSection,
}

impl Default for EnrichSection {
    fn default() -> Self {
        EnrichSection {
            backend: "none".into(),
            max_usd_per_run: 0.50,
            allow_egress_default: false,
            respect_do_not_share: true,
            timeout_secs: 20,
            treg: TregSection::default(),
            ownkeys: OwnKeysSection::default(),
        }
    }
}

/// `[enrich.treg]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TregSection {
    /// Router endpoint.
    pub endpoint: String,
    /// Env var holding the token.
    pub token_env: String,
}

impl Default for TregSection {
    fn default() -> Self {
        TregSection {
            endpoint: "https://treg.to".into(),
            token_env: "OPENSTANCE_TREG_TOKEN".into(),
        }
    }
}

/// `[enrich.ownkeys]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OwnKeysSection {
    /// Provider preference.
    pub order: Vec<String>,
    /// Micro-USD per op.
    pub prices: BTreeMap<String, u64>,
    /// Optional generic verifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifySection>,
}

impl Default for OwnKeysSection {
    fn default() -> Self {
        OwnKeysSection {
            order: vec!["pdl".into(), "apollo".into(), "clearbit".into()],
            prices: BTreeMap::new(),
            verify: None,
        }
    }
}

/// `[enrich.ownkeys.verify]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifySection {
    /// URL with `{email}`.
    pub url: String,
    /// Env var holding the key.
    #[serde(default)]
    pub key_env: Option<String>,
    /// Header name.
    #[serde(default = "default_key_header")]
    pub key_header: String,
}

fn default_key_header() -> String {
    "Authorization".into()
}

/// `[me]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MeSection {
    /// Own handles as `kind:value`.
    pub identities: Vec<String>,
}

/// `[import]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportSection {
    /// Headers only by default.
    pub email_bodies: bool,
    /// Larger mbox messages are skipped after the header.
    pub max_message_mb: u32,
    /// ISO 3166 alpha-2 for phones without `+`; empty = none.
    pub default_region: String,
    /// IANA zone for naive timestamps; empty = system local.
    pub timezone: String,
    /// Larger calendar events are skipped.
    pub ics_max_attendees: u32,
    /// Named column maps (Phase 5).
    pub csv: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for ImportSection {
    fn default() -> Self {
        ImportSection {
            email_bodies: false,
            max_message_mb: 50,
            default_region: String::new(),
            timezone: String::new(),
            ics_max_attendees: 50,
            csv: BTreeMap::new(),
        }
    }
}

/// `[share]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareSection {
    /// age identity file.
    pub keypair: String,
    /// `Nd | Nw | Nm | Ny | never`.
    pub default_ttl: String,
    /// Claimed owner label.
    pub owner_label: String,
    /// ASCII-armored `.ost`.
    pub armor: bool,
    /// Decrypted size limit for `view import`.
    pub max_import_mb: u32,
    /// Score at or above this is `strong`.
    pub bucket_strong: f64,
    /// At or above this is `medium`.
    pub bucket_medium: f64,
    /// `alias -> age1...`.
    pub recipients: BTreeMap<String, String>,
}

impl Default for ShareSection {
    fn default() -> Self {
        ShareSection {
            keypair: "~/.openstance/age.key".into(),
            default_ttl: "30d".into(),
            owner_label: String::new(),
            armor: false,
            max_import_mb: 100,
            bucket_strong: 0.60,
            bucket_medium: 0.30,
            recipients: BTreeMap::new(),
        }
    }
}

/// `[serve]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServeSection {
    /// Bind address.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Bearer token file.
    pub token_path: String,
    /// Serve the embedded UI.
    pub ui: bool,
    /// Allowed CORS origins.
    pub cors_origins: Vec<String>,
    /// Upload limit.
    pub max_upload_mb: u32,
}

impl Default for ServeSection {
    fn default() -> Self {
        ServeSection {
            host: "127.0.0.1".into(),
            port: 7411,
            token_path: "~/.openstance/api.token".into(),
            ui: true,
            cors_origins: Vec::new(),
            max_upload_mb: 2048,
        }
    }
}

/// `[mcp]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSection {
    /// Force `allow_egress = false` and refuse edits.
    pub read_only: bool,
    /// `search_people` default limit.
    pub default_limit: u32,
}

impl Default for McpSection {
    fn default() -> Self {
        McpSection {
            read_only: false,
            default_limit: 20,
        }
    }
}

/// `[playbooks.*]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PlaybooksSection {
    /// `[playbooks.path]`.
    pub path: PathPlaybook,
    /// `[playbooks.refresh]`.
    pub refresh: RefreshPlaybook,
    /// `[playbooks.watch]`.
    pub watch: WatchPlaybook,
    /// `[playbooks.reconnect]`.
    pub reconnect: ReconnectPlaybook,
    /// `[playbooks.brief]`.
    pub brief: BriefPlaybook,
    /// `[playbooks.verify]`.
    pub verify: VerifyPlaybook,
}

/// `[playbooks.path]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathPlaybook {
    /// Paths returned.
    pub limit: u32,
}
impl Default for PathPlaybook {
    fn default() -> Self {
        PathPlaybook { limit: 3 }
    }
}

/// `[playbooks.refresh]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefreshPlaybook {
    /// Minimum interactions.
    pub min_interactions: u32,
    /// Max people per run.
    pub limit: u32,
}
impl Default for RefreshPlaybook {
    fn default() -> Self {
        RefreshPlaybook {
            min_interactions: 5,
            limit: 50,
        }
    }
}

/// `[playbooks.watch]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchPlaybook {
    /// Highlighted companies.
    pub targets: Vec<String>,
    /// Profile checks with `--check-profiles`.
    pub top_n: u32,
}
impl Default for WatchPlaybook {
    fn default() -> Self {
        WatchPlaybook {
            targets: Vec::new(),
            top_n: 100,
        }
    }
}

/// `[playbooks.reconnect]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReconnectPlaybook {
    /// Results.
    pub limit: u32,
    /// F threshold.
    pub min_frequency: f64,
    /// R threshold.
    pub max_recency: f64,
}
impl Default for ReconnectPlaybook {
    fn default() -> Self {
        ReconnectPlaybook {
            limit: 10,
            min_frequency: 0.5,
            max_recency: 0.2,
        }
    }
}

/// `[playbooks.brief]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BriefPlaybook {
    /// Allow prose rewriting by an API model.
    pub prose: bool,
}

/// `[playbooks.verify]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyPlaybook {
    /// Skip re-verification within this window.
    pub fresh_days: u32,
}
impl Default for VerifyPlaybook {
    fn default() -> Self {
        VerifyPlaybook { fresh_days: 30 }
    }
}

/// `[log]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogSection {
    /// stderr level.
    pub level: String,
    /// Log file path or `""`.
    pub file: String,
    /// File level.
    pub file_level: String,
    /// `text | json`.
    pub format: String,
}

impl Default for LogSection {
    fn default() -> Self {
        LogSection {
            level: "warn".into(),
            file: "~/.openstance/logs/openstance.log".into(),
            file_level: "info".into(),
            format: "text".into(),
        }
    }
}

/// `[sync]` (Phase 5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SyncSection {
    /// Named remotes.
    pub remotes: BTreeMap<String, SyncRemote>,
}

/// `[sync.remotes.<name>]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncRemote {
    /// Remote URL.
    pub url: String,
    /// Used when `--remote` is omitted.
    #[serde(default)]
    pub default: bool,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Inputs to [`load`]. Use [`LoadOptions::from_process_env`] in the binary;
/// tests pass an explicit environment.
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// `--config`.
    pub config_path: Option<PathBuf>,
    /// `--db`.
    pub db_path: Option<PathBuf>,
    /// Environment variables (only `OPENSTANCE*` and `HOME` are read).
    pub env: BTreeMap<String, String>,
    /// `config --strict`: unknown keys are errors.
    pub strict: bool,
}

impl LoadOptions {
    /// Capture the process environment.
    pub fn from_process_env() -> Self {
        LoadOptions {
            env: std::env::vars()
                .filter(|(k, _)| k.starts_with("OPENSTANCE") || k == "HOME" || k == "USERPROFILE")
                .collect(),
            ..LoadOptions::default()
        }
    }
}

/// Resolved file locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$OPENSTANCE_HOME`, default `~/.openstance`.
    pub home: PathBuf,
    /// The config file that was (or would be) read.
    pub config_file: PathBuf,
    /// The database file.
    pub db: PathBuf,
}

/// The result of [`load`].
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    /// Effective configuration.
    pub config: Config,
    /// Resolved paths.
    pub paths: Paths,
    /// Non-fatal problems (unknown keys, normalized weights).
    pub warnings: Vec<String>,
    /// Dotted key -> origin, for every leaf key.
    pub origins: BTreeMap<String, Origin>,
    /// Whether the config file existed.
    pub file_found: bool,
}

/// Load config: defaults, then the file, then `OPENSTANCE__*` variables, then
/// flags in `opts`. Validates the result.
pub fn load(opts: &LoadOptions) -> Result<LoadedConfig> {
    let user_home = opts
        .env
        .get("HOME")
        .or_else(|| opts.env.get("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    let home = match opts.env.get("OPENSTANCE_HOME").filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => user_home
            .as_deref()
            .map_or_else(|| PathBuf::from(".openstance"), |u| u.join(".openstance")),
    };
    let config_file = match (
        &opts.config_path,
        opts.env.get("OPENSTANCE_CONFIG").filter(|c| !c.is_empty()),
    ) {
        (Some(p), _) => p.clone(),
        (None, Some(p)) => expand_path(p, &home, user_home.as_deref()),
        (None, None) => home.join("config.toml"),
    };

    let (file_value, file_found) = match std::fs::read_to_string(&config_file) {
        Ok(text) => (parse_toml(&text, &config_file.display().to_string())?, true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (toml::Table::new(), false),
        Err(e) => return Err(Error::io("reading", &config_file, e)),
    };

    let mut merged = file_value.clone();
    let mut env_paths: Vec<String> = Vec::new();
    for (k, v) in &opts.env {
        let Some(rest) = k.strip_prefix("OPENSTANCE__") else {
            continue;
        };
        let path: Vec<String> = rest.split("__").map(|s| s.to_ascii_lowercase()).collect();
        if path.iter().any(String::is_empty) {
            continue;
        }
        set_path(&mut merged, &path, parse_env_value(v));
        env_paths.push(path.join("."));
    }
    let mut flag_paths: Vec<String> = Vec::new();
    if let Some(db) = opts.env.get("OPENSTANCE_DB").filter(|d| !d.is_empty()) {
        set_path(
            &mut merged,
            &["db".into(), "path".into()],
            toml::Value::String(db.clone()),
        );
        env_paths.push("db.path".into());
    }
    if let Some(db) = &opts.db_path {
        set_path(
            &mut merged,
            &["db".into(), "path".into()],
            toml::Value::String(db.display().to_string()),
        );
        flag_paths.push("db.path".into());
    }

    let (config, warnings) = finish(merged, opts.strict)?;

    let mut origins = BTreeMap::new();
    let defaults = default_table();
    for leaf in leaves(&defaults, "") {
        origins.insert(leaf, Origin::Default);
    }
    for leaf in leaves(&file_value, "") {
        origins.insert(leaf, Origin::File);
    }
    for p in env_paths {
        origins.insert(p, Origin::Env);
    }
    for p in flag_paths {
        origins.insert(p, Origin::Flag);
    }

    let db = config.db_path(&home, user_home.as_deref());
    Ok(LoadedConfig {
        config,
        paths: Paths {
            home,
            config_file,
            db,
        },
        warnings,
        origins,
        file_found,
    })
}

/// Tables whose keys are user-chosen (never "unknown").
const OPEN_TABLES: &[&str] = &[
    "score.weights",
    "resolution.company_aliases",
    "enrich.ownkeys.prices",
    "enrich.ownkeys.verify",
    "share.recipients",
    "import.csv",
    "sync.remotes",
];

/// Keys that are valid but absent from the serialized defaults.
const OPTIONAL_KEYS: &[&str] = &[
    "score.preset",
    "score.weights",
    "model.openai.dimensions",
    "enrich.ownkeys.verify",
];

fn parse_toml(text: &str, origin: &str) -> Result<toml::Table> {
    text.parse::<toml::Table>().map_err(|e| Error::Config {
        key: origin.to_string(),
        message: e.message().to_string(),
    })
}

fn default_table() -> toml::Table {
    toml::Table::try_from(Config::default()).unwrap_or_default()
}

fn finish(value: toml::Table, strict: bool) -> Result<(Config, Vec<String>)> {
    let unknown = unknown_keys(&value, &default_table(), "");
    if strict {
        if let Some(k) = unknown.first() {
            return Err(Error::Config {
                key: k.clone(),
                message: "unknown key".into(),
            });
        }
    }
    let config: Config = toml::Value::Table(value)
        .try_into()
        .map_err(|e: toml::de::Error| Error::Config {
            key: "config".into(),
            message: e.message().to_string(),
        })?;
    config.validate()?;
    let warnings = unknown
        .into_iter()
        .map(|k| format!("unknown config key `{k}` (ignored)"))
        .collect();
    Ok((config, warnings))
}

fn join(prefix: &str, k: &str) -> String {
    if prefix.is_empty() {
        k.to_string()
    } else {
        format!("{prefix}.{k}")
    }
}

fn unknown_keys(user: &toml::Table, defaults: &toml::Table, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in user {
        let path = join(prefix, k);
        if OPEN_TABLES.contains(&path.as_str()) {
            continue;
        }
        match defaults.get(k) {
            Some(toml::Value::Table(dt)) => {
                if let toml::Value::Table(ut) = v {
                    out.extend(unknown_keys(ut, dt, &path));
                }
            }
            Some(_) => {}
            None if OPTIONAL_KEYS.contains(&path.as_str()) => {}
            None => out.push(path),
        }
    }
    out
}

fn leaves(t: &toml::Table, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in t {
        let path = join(prefix, k);
        match v {
            toml::Value::Table(sub) if !OPEN_TABLES.contains(&path.as_str()) => {
                out.extend(leaves(sub, &path))
            }
            _ => out.push(path),
        }
    }
    out
}

fn set_path(t: &mut toml::Table, path: &[String], value: toml::Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut cur = t;
    for p in parents {
        let entry = cur
            .entry(p.clone())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        let toml::Value::Table(next) = entry else {
            return;
        };
        cur = next;
    }
    cur.insert(last.clone(), value);
}

/// Env values are TOML scalars or arrays (`120`, `true`, `'["a","b"]'`);
/// anything that does not parse is a plain string.
fn parse_env_value(v: &str) -> toml::Value {
    format!("v = {v}")
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(v.to_string()))
}

/// Parse a config file's text (no env, no paths). Returns the config and
/// warnings for unknown keys. Validates the result.
pub fn parse_str(text: &str, strict: bool) -> Result<(Config, Vec<String>)> {
    finish(parse_toml(text, "config.toml")?, strict)
}

/// The `config.toml` that `openstance init` writes: every key commented out
/// at its default. `me` handles, when given, are written uncommented.
pub fn init_file_contents(me: &[String]) -> String {
    let mut out = String::new();
    let mut section = String::new();
    for line in REFERENCE_TOML.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            section = trimmed
                .trim_start_matches('[')
                .split(']')
                .next()
                .unwrap_or_default()
                .to_string();
            out.push_str(line);
        } else if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(line);
        } else if section == "me" && trimmed.starts_with("identities") && !me.is_empty() {
            let list: Vec<String> = me.iter().map(|h| format!("{h:?}")).collect();
            out.push_str(&format!("identities = [{}]", list.join(", ")));
        } else {
            out.push_str("# ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

impl Config {
    /// Range and enum checks (configuration.md section 4). Errors name the key.
    pub fn validate(&self) -> Result<()> {
        fn bad(key: &str, message: impl Into<String>) -> Error {
            Error::Config {
                key: key.to_string(),
                message: message.into(),
            }
        }
        fn positive(key: &str, v: f64) -> Result<()> {
            if v.is_finite() && v > 0.0 {
                Ok(())
            } else {
                Err(bad(key, format!("must be > 0, got {v}")))
            }
        }
        fn within(key: &str, v: f64, lo: f64, hi: f64) -> Result<()> {
            if v.is_finite() && (lo..=hi).contains(&v) {
                Ok(())
            } else {
                Err(bad(key, format!("must be between {lo} and {hi}, got {v}")))
            }
        }
        fn one_of(key: &str, v: &str, allowed: &[&str]) -> Result<()> {
            if allowed.contains(&v) {
                Ok(())
            } else {
                Err(bad(
                    key,
                    format!("must be one of {}, got {v:?}", allowed.join(", ")),
                ))
            }
        }

        within(
            "db.busy_timeout_ms",
            f64::from(self.db.busy_timeout_ms),
            0.0,
            60_000.0,
        )?;

        let s = &self.score;
        if let Some(p) = &s.preset {
            one_of(
                "score.preset",
                p,
                &["default", "founder", "recruiter", "seller", "community"],
            )?;
        }
        if let Some(w) = &s.weights {
            if s.preset.is_some() {
                return Err(bad(
                    "score.weights",
                    "set either score.preset or score.weights, not both",
                ));
            }
            let all = [w.recency, w.frequency, w.reciprocity, w.meetings, w.breadth];
            if all.iter().any(|x| !x.is_finite() || *x < 0.0) || all.iter().sum::<f64>() <= 0.0 {
                return Err(bad(
                    "score.weights",
                    "weights must be >= 0 with a positive sum",
                ));
            }
        }
        positive("score.half_life_days", s.half_life_days)?;
        positive("score.frequency_scale", s.frequency_scale)?;
        positive("score.meetings_scale", s.meetings_scale)?;
        positive("score.tie_scale", s.tie_scale)?;
        if !(s.thread_start_multiplier.is_finite() && s.thread_start_multiplier >= 1.0) {
            return Err(bad("score.thread_start_multiplier", "must be >= 1"));
        }
        if s.tie_max_participants < 2 {
            return Err(bad("score.tie_max_participants", "must be >= 2"));
        }
        if s.bulk.min_messages < 1 {
            return Err(bad("score.bulk.min_messages", "must be >= 1"));
        }
        within(
            "score.bulk.list_unsubscribe_ratio",
            s.bulk.list_unsubscribe_ratio,
            0.0,
            1.0,
        )?;

        let r = &self.resolution;
        within(
            "resolution.suggest_jaro_winkler",
            r.suggest_jaro_winkler,
            0.80,
            1.0,
        )?;
        within(
            "resolution.common_names_top_n",
            f64::from(r.common_names_top_n),
            0.0,
            50_000.0,
        )?;
        if r.large_company_threshold < 2 {
            return Err(bad("resolution.large_company_threshold", "must be >= 2"));
        }

        let q = &self.search;
        within(
            "search.default_limit",
            f64::from(q.default_limit),
            1.0,
            1000.0,
        )?;
        within(
            "search.candidates_fts",
            f64::from(q.candidates_fts),
            10.0,
            5000.0,
        )?;
        within(
            "search.candidates_vec",
            f64::from(q.candidates_vec),
            10.0,
            5000.0,
        )?;
        within("search.text_weight", q.text_weight, 0.0, 1.0)?;
        within("search.relationship_lift", q.relationship_lift, 0.0, 1.0)?;
        within(
            "search.evidence_per_result",
            f64::from(q.evidence_per_result),
            0.0,
            20.0,
        )?;

        let m = &self.model;
        if m.embedder.starts_with("anthropic:") {
            return Err(bad(
                "model.embedder",
                "anthropic has no embeddings endpoint",
            ));
        }
        within(
            "model.timeout_local_ms",
            f64::from(m.timeout_local_ms),
            100.0,
            300_000.0,
        )?;
        within(
            "model.timeout_remote_ms",
            f64::from(m.timeout_remote_ms),
            100.0,
            300_000.0,
        )?;
        within("model.rerank_top_n", f64::from(m.rerank_top_n), 1.0, 200.0)?;
        one_of(
            "model.openai.json_mode",
            &m.openai.json_mode,
            &["schema", "object", "prompt"],
        )?;

        let e = &self.enrich;
        one_of("enrich.backend", &e.backend, &["none", "treg", "ownkeys"])?;
        if !(e.max_usd_per_run.is_finite() && e.max_usd_per_run >= 0.0) {
            return Err(bad("enrich.max_usd_per_run", "must be >= 0"));
        }
        within("enrich.timeout_secs", f64::from(e.timeout_secs), 1.0, 120.0)?;

        for h in &self.me.identities {
            let ok = h.split_once(':').is_some_and(|(k, v)| {
                openstance_import::HandleKind::parse(k.trim()).is_some() && !v.trim().is_empty()
            });
            if !ok {
                return Err(bad(
                    "me.identities",
                    format!("entries must look like \"email:you@example.com\", got {h:?}"),
                ));
            }
        }

        let i = &self.import;
        within(
            "import.max_message_mb",
            f64::from(i.max_message_mb),
            1.0,
            1000.0,
        )?;
        if i.ics_max_attendees < 2 {
            return Err(bad("import.ics_max_attendees", "must be >= 2"));
        }
        if !(i.default_region.is_empty()
            || (i.default_region.len() == 2
                && i.default_region.chars().all(|c| c.is_ascii_alphabetic())))
        {
            return Err(bad(
                "import.default_region",
                "must be an ISO 3166 alpha-2 code like \"US\"",
            ));
        }

        let sh = &self.share;
        within("share.bucket_strong", sh.bucket_strong, 0.0, 1.0)?;
        within(
            "share.bucket_medium",
            sh.bucket_medium,
            0.0,
            sh.bucket_strong,
        )?;
        within(
            "share.max_import_mb",
            f64::from(sh.max_import_mb),
            1.0,
            2048.0,
        )?;

        if self.serve.port == 0 {
            return Err(bad("serve.port", "must be between 1 and 65535"));
        }
        within(
            "serve.max_upload_mb",
            f64::from(self.serve.max_upload_mb),
            1.0,
            20_000.0,
        )?;

        let levels = ["error", "warn", "info", "debug", "trace"];
        one_of("log.level", &self.log.level, &levels)?;
        one_of("log.file_level", &self.log.file_level, &levels)?;
        one_of("log.format", &self.log.format, &["text", "json"])?;
        Ok(())
    }

    /// The effective configuration as TOML (`openstance config`).
    pub fn to_toml(&self) -> String {
        let mut c = self.clone();
        if c.score.preset.is_none() && c.score.weights.is_none() {
            c.score.preset = Some("default".into());
        }
        toml::to_string_pretty(&c).unwrap_or_default()
    }

    /// Database path, expanded against `home` (the openstance home) and
    /// `user_home` (`$HOME`, for `~/` paths outside `~/.openstance`).
    pub fn db_path(&self, home: &Path, user_home: Option<&Path>) -> PathBuf {
        expand_path(&self.db.path, home, user_home)
    }

    /// `[me] identities` parsed into `(kind, value)` pairs.
    pub fn me_identities(&self) -> Vec<(openstance_import::HandleKind, String)> {
        self.me
            .identities
            .iter()
            .filter_map(|s| {
                let (k, v) = s.split_once(':')?;
                Some((
                    openstance_import::HandleKind::parse(k.trim())?,
                    v.trim().to_string(),
                ))
            })
            .collect()
    }
}

/// Expand `~`, `$OPENSTANCE_HOME` and relative paths. `~/.openstance/x`
/// resolves inside `home`, so `OPENSTANCE_HOME` relocates every default path;
/// other `~/x` paths resolve against `user_home` (falling back to `home`).
pub fn expand_path(p: &str, home: &Path, user_home: Option<&Path>) -> PathBuf {
    fn under(base: &Path, rest: &str) -> PathBuf {
        let rest = rest.trim_start_matches(['/', '\\']);
        if rest.is_empty() {
            base.to_path_buf()
        } else {
            base.join(rest)
        }
    }
    if let Some(rest) = p.strip_prefix("~/.openstance") {
        if rest.is_empty() || rest.starts_with(['/', '\\']) {
            return under(home, rest);
        }
    }
    if let Some(rest) = p.strip_prefix("$OPENSTANCE_HOME") {
        return under(home, rest);
    }
    if p == "~" {
        return user_home.unwrap_or(home).to_path_buf();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return under(user_home.unwrap_or(home), rest);
    }
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn empty_text_is_exactly_the_defaults() {
        let (c, w) = parse_str("", false).unwrap();
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
    }

    #[test]
    fn defaults_match_the_configuration_doc() {
        let c = Config::default();
        assert_eq!(c.enrich.backend, "none");
        assert!(!c.enrich.allow_egress_default);
        assert_eq!(c.enrich.max_usd_per_run, 0.50);
        // The hosted router (dev/treg/INTEGRATION.md); api.treg.to does not exist.
        assert_eq!(c.enrich.treg.endpoint, "https://treg.to");
        assert_eq!(c.enrich.treg.token_env, "OPENSTANCE_TREG_TOKEN");
        assert!(!c.import.email_bodies);
        assert_eq!(c.score.half_life_days, 180.0);
        assert_eq!(c.score.effective_preset(), "default");
        assert_eq!(c.score.bulk.min_messages, 50);
        assert_eq!(c.score.tie_max_participants, 20);
        assert_eq!(c.resolution.suggest_jaro_winkler, 0.92);
        assert_eq!(c.resolution.large_company_threshold, 50);
        assert_eq!(c.search.default_limit, 30);
        assert_eq!(c.search.candidates_fts, 200);
        assert_eq!(c.model.parser, "rules");
        assert_eq!(c.model.reranker, "none");
        assert_eq!(c.db.path, "~/.openstance/graph.db");
        assert_eq!(c.db.busy_timeout_ms, 5000);
        assert_eq!(c.serve.port, 7411);
        assert_eq!(c.serve.host, "127.0.0.1");
        assert_eq!(c.mcp.default_limit, 20);
        assert_eq!(c.log.level, "warn");
        assert!(c.me.identities.is_empty());
    }

    #[test]
    fn reference_file_parses_to_the_defaults_without_warnings() {
        let (c, w) = parse_str(REFERENCE_TOML, true).unwrap();
        let mut want = Config::default();
        want.score.preset = Some("default".into());
        assert_eq!(c, want);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn init_file_is_fully_commented_and_equals_defaults() {
        let text = init_file_contents(&[]);
        let (c, w) = parse_str(&text, true).unwrap();
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
        assert!(text.contains("# half_life_days = 180"));
        assert!(text.contains("[score]"));
    }

    #[test]
    fn init_file_records_me_handles() {
        let text = init_file_contents(&["email:owner@example.com".to_string()]);
        let (c, _) = parse_str(&text, true).unwrap();
        assert_eq!(c.me.identities, vec!["email:owner@example.com".to_string()]);
        assert_eq!(
            c.me_identities(),
            vec![(
                openstance_import::HandleKind::Email,
                "owner@example.com".to_string()
            )]
        );
    }

    #[test]
    fn unknown_keys_warn_and_strict_mode_errors() {
        let text = "[score]\nhalf_life_days = 90\nhalf_lief_days = 1\n[nonsense]\nx = 1\n";
        let (c, w) = parse_str(text, false).unwrap();
        assert_eq!(c.score.half_life_days, 90.0);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().any(|m| m.contains("score.half_lief_days")));
        assert!(w.iter().any(|m| m.contains("nonsense")));
        let err = parse_str(text, true).unwrap_err();
        assert!(matches!(err, Error::Config { .. }));
    }

    #[test]
    fn open_tables_accept_any_keys() {
        let text = r#"
[resolution]
company_aliases = { "stripe inc" = "stripe" }
[share.recipients]
alice = "age1abc"
[enrich.ownkeys.prices]
profile = 10000
[import.csv.luma]
name = "Full Name"
"#;
        let (c, w) = parse_str(text, true).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            c.resolution
                .company_aliases
                .get("stripe inc")
                .map(String::as_str),
            Some("stripe")
        );
        assert_eq!(c.enrich.ownkeys.prices.get("profile"), Some(&10000));
    }

    #[test]
    fn type_errors_are_errors() {
        assert!(matches!(
            parse_str("[score]\nhalf_life_days = \"long\"\n", false),
            Err(Error::Config { .. })
        ));
        assert!(matches!(
            parse_str("not toml at all [", false),
            Err(Error::Config { .. })
        ));
    }

    #[test]
    fn out_of_range_values_name_the_key() {
        for (text, key) in [
            ("[score]\nhalf_life_days = 0\n", "score.half_life_days"),
            ("[score]\npreset = \"vibes\"\n", "score.preset"),
            (
                "[score.bulk]\nlist_unsubscribe_ratio = 1.5\n",
                "score.bulk.list_unsubscribe_ratio",
            ),
            (
                "[score]\ntie_max_participants = 1\n",
                "score.tie_max_participants",
            ),
            (
                "[resolution]\nsuggest_jaro_winkler = 0.5\n",
                "resolution.suggest_jaro_winkler",
            ),
            ("[search]\ndefault_limit = 0\n", "search.default_limit"),
            ("[search]\ntext_weight = 2\n", "search.text_weight"),
            ("[enrich]\nbackend = \"magic\"\n", "enrich.backend"),
            ("[enrich]\nmax_usd_per_run = -1\n", "enrich.max_usd_per_run"),
            ("[log]\nlevel = \"loud\"\n", "log.level"),
            ("[me]\nidentities = [\"nonsense\"]\n", "me.identities"),
            (
                "[model]\nembedder = \"anthropic:claude\"\n",
                "model.embedder",
            ),
        ] {
            match parse_str(text, false) {
                Err(Error::Config { key: k, .. }) => assert_eq!(k, key, "{text}"),
                other => panic!("{text}: expected config error, got {other:?}"),
            }
        }
    }

    #[test]
    fn preset_and_weights_are_mutually_exclusive_and_weights_need_all_keys() {
        let both = "[score]\npreset = \"founder\"\nweights = { recency = 0.3, frequency = 0.15, reciprocity = 0.3, meetings = 0.2, breadth = 0.05 }\n";
        assert!(
            matches!(parse_str(both, false), Err(Error::Config { key, .. }) if key == "score.weights")
        );
        let missing = "[score]\nweights = { recency = 1.0 }\n";
        assert!(parse_str(missing, false).is_err());
        let ok = "[score]\nweights = { recency = 0.3, frequency = 0.15, reciprocity = 0.3, meetings = 0.2, breadth = 0.05 }\n";
        let (c, _) = parse_str(ok, false).unwrap();
        assert_eq!(c.score.effective_preset(), "custom");
    }

    #[test]
    fn load_with_no_file_gives_defaults_and_home_paths() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("osh");
        let l = load(&LoadOptions {
            env: env(&[("OPENSTANCE_HOME", home.to_str().unwrap())]),
            ..LoadOptions::default()
        })
        .unwrap();
        assert!(!l.file_found);
        assert_eq!(l.config, Config::default());
        assert_eq!(l.paths.home, home);
        assert_eq!(l.paths.config_file, home.join("config.toml"));
        assert_eq!(l.paths.db, home.join("graph.db"));
        assert_eq!(
            l.origins.get("score.half_life_days"),
            Some(&Origin::Default)
        );
    }

    #[test]
    fn load_layers_file_then_env_then_flags() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        std::fs::write(
            home.join("config.toml"),
            "[score]\nhalf_life_days = 90\npreset = \"founder\"\n[enrich]\nbackend = \"treg\"\n",
        )
        .unwrap();
        let l = load(&LoadOptions {
            env: env(&[
                ("OPENSTANCE_HOME", home.to_str().unwrap()),
                ("OPENSTANCE__SCORE__HALF_LIFE_DAYS", "120"),
                (
                    "OPENSTANCE__ME__IDENTITIES",
                    r#"["email:owner@example.com"]"#,
                ),
            ]),
            db_path: Some(home.join("other.db")),
            ..LoadOptions::default()
        })
        .unwrap();
        assert!(l.file_found);
        assert_eq!(l.config.score.half_life_days, 120.0);
        assert_eq!(l.config.score.preset.as_deref(), Some("founder"));
        assert_eq!(l.config.enrich.backend, "treg");
        assert_eq!(
            l.config.me.identities,
            vec!["email:owner@example.com".to_string()]
        );
        assert_eq!(l.origins.get("score.half_life_days"), Some(&Origin::Env));
        assert_eq!(l.origins.get("score.preset"), Some(&Origin::File));
        assert_eq!(
            l.origins.get("enrich.max_usd_per_run"),
            Some(&Origin::Default)
        );
        assert_eq!(l.origins.get("db.path"), Some(&Origin::Flag));
        assert_eq!(l.paths.db, home.join("other.db"));
    }

    #[test]
    fn openstance_db_and_config_env_vars_are_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("custom.toml");
        std::fs::write(&cfg, "[search]\ndefault_limit = 5\n").unwrap();
        let l = load(&LoadOptions {
            env: env(&[
                ("OPENSTANCE_HOME", dir.path().to_str().unwrap()),
                ("OPENSTANCE_CONFIG", cfg.to_str().unwrap()),
                ("OPENSTANCE_DB", dir.path().join("x.db").to_str().unwrap()),
            ]),
            ..LoadOptions::default()
        })
        .unwrap();
        assert_eq!(l.config.search.default_limit, 5);
        assert_eq!(l.paths.db, dir.path().join("x.db"));
        assert_eq!(l.origins.get("db.path"), Some(&Origin::Env));
    }

    #[test]
    fn home_defaults_to_dot_openstance_under_home() {
        let l = load(&LoadOptions {
            env: env(&[("HOME", "/nonexistent-home-for-test")]),
            ..LoadOptions::default()
        })
        .unwrap();
        assert_eq!(
            l.paths.home,
            PathBuf::from("/nonexistent-home-for-test/.openstance")
        );
        assert_eq!(
            l.paths.db,
            PathBuf::from("/nonexistent-home-for-test/.openstance/graph.db")
        );
    }

    #[test]
    fn expand_path_rules() {
        let home = Path::new("/h/.os");
        let user = Some(Path::new("/users/me"));
        assert_eq!(
            expand_path("~/.openstance/graph.db", home, user),
            PathBuf::from("/h/.os/graph.db")
        );
        assert_eq!(
            expand_path("~/.openstance", home, user),
            PathBuf::from("/h/.os")
        );
        assert_eq!(
            expand_path("~/data/g.db", home, user),
            PathBuf::from("/users/me/data/g.db")
        );
        assert_eq!(
            expand_path("$OPENSTANCE_HOME/x.db", home, user),
            PathBuf::from("/h/.os/x.db")
        );
        assert_eq!(
            expand_path("rel.db", home, user),
            PathBuf::from("/h/.os/rel.db")
        );
        assert_eq!(
            expand_path("/abs/g.db", home, user),
            PathBuf::from("/abs/g.db")
        );
    }

    #[test]
    fn to_toml_round_trips() {
        let mut c = Config::default();
        c.score.half_life_days = 90.0;
        c.me.identities = vec!["email:owner@example.com".into()];
        let text = c.to_toml();
        assert!(
            text.contains("preset = \"default\""),
            "effective preset is printed:\n{text}"
        );
        let (back, w) = parse_str(&text, true).unwrap();
        assert!(w.is_empty());
        let mut want = c.clone();
        want.score.preset = Some("default".into());
        assert_eq!(back, want);
    }
}

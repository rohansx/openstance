//! One writer and N readers over the same `graph.db` (P4-API-01).
//!
//! Core is synchronous, so every call runs on tokio's blocking pool. Writes
//! (edits, imports, playbooks that log egress, MCP tool calls) go through a
//! single writer connection behind a mutex; reads (search, person detail,
//! review list, stats) check out one of N reader connections, so a long
//! import never blocks browsing. SQLite in WAL mode lets the readers see the
//! last committed state while the writer works.
//!
//! The writer holds the binary's providers (importers, enricher, models).
//! Readers share the same model instances through `Arc` (one embedding model
//! in memory, not N), get no importers and the null enricher: nothing that
//! can send a request runs on a reader.

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use openstance_core::enrich::NullEnricher;
use openstance_core::model::{
    Candidate, EmbedKind, Embedder, Models, Query, QueryParser, Ranked, Reranker,
};
use openstance_core::{Clock, Config, Graph, Providers};

use crate::error::ApiError;

/// The writer graph, shared with the MCP-over-HTTP handler.
pub type SharedGraph = Arc<Mutex<Graph>>;

/// Makes the clock of each connection (`--now` in tests).
pub type ClockFn = Arc<dyn Fn() -> Box<dyn Clock> + Send + Sync>;

/// The connection pool.
#[derive(Clone)]
pub struct GraphPool {
    writer: SharedGraph,
    readers: Arc<Readers>,
}

impl std::fmt::Debug for GraphPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphPool").finish_non_exhaustive()
    }
}

struct Readers {
    free: Mutex<Vec<Graph>>,
    ready: Condvar,
}

/// Puts a reader back when the call ends, also on panic.
struct Lease<'a> {
    graph: Option<Graph>,
    pool: &'a Readers,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(g) = self.graph.take() {
            self.pool
                .free
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(g);
            self.pool.ready.notify_one();
        }
    }
}

impl GraphPool {
    /// Open the writer (with `providers`) and `readers` reader connections
    /// (at least one) on `db`.
    pub fn open(
        cfg: Config,
        db: &Path,
        providers: Providers,
        clock: ClockFn,
        readers: usize,
    ) -> openstance_core::Result<GraphPool> {
        let Providers {
            importers,
            models,
            enricher,
        } = providers;
        let embedder: Arc<dyn Embedder> = Arc::from(models.embedder);
        let parser: Arc<dyn QueryParser> = Arc::from(models.parser);
        let reranker: Arc<dyn Reranker> = Arc::from(models.reranker);
        let writer = Graph::open(
            cfg.clone(),
            db,
            Providers {
                importers,
                models: shared_models(&embedder, &parser, &reranker),
                enricher,
            },
            clock(),
        )?;
        let mut free = Vec::with_capacity(readers.max(1));
        for _ in 0..readers.max(1) {
            free.push(Graph::open(
                cfg.clone(),
                db,
                reader_providers(shared_models(&embedder, &parser, &reranker)),
                clock(),
            )?);
        }
        Ok(GraphPool {
            writer: Arc::new(Mutex::new(writer)),
            readers: Arc::new(Readers {
                free: Mutex::new(free),
                ready: Condvar::new(),
            }),
        })
    }

    /// The writer, for the MCP handler (which locks it per tool call).
    pub fn writer(&self) -> SharedGraph {
        Arc::clone(&self.writer)
    }

    /// Run `f` on a reader connection.
    pub async fn read<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&Graph) -> Result<T, ApiError> + Send + 'static,
    {
        let readers = Arc::clone(&self.readers);
        joined(
            tokio::task::spawn_blocking(move || {
                let mut free = readers.free.lock().unwrap_or_else(PoisonError::into_inner);
                let graph = loop {
                    match free.pop() {
                        Some(g) => break g,
                        None => {
                            free = readers
                                .ready
                                .wait(free)
                                .unwrap_or_else(PoisonError::into_inner);
                        }
                    }
                };
                drop(free);
                let lease = Lease {
                    graph: Some(graph),
                    pool: &readers,
                };
                f(lease.graph.as_ref().expect("leased graph"))
            })
            .await,
        )
    }

    /// Run `f` on the writer connection.
    pub async fn write<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Graph) -> Result<T, ApiError> + Send + 'static,
    {
        let writer = Arc::clone(&self.writer);
        joined(
            tokio::task::spawn_blocking(move || {
                // A call that panicked poisons the lock; the graph is still
                // consistent (its transaction rolled back on unwind).
                let mut g = writer.lock().unwrap_or_else(PoisonError::into_inner);
                f(&mut g)
            })
            .await,
        )
    }
}

fn joined<T>(r: Result<Result<T, ApiError>, tokio::task::JoinError>) -> Result<T, ApiError> {
    match r {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "request handler panicked");
            Err(ApiError::internal(
                "the request failed unexpectedly; see the log",
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Shared model instances
// ---------------------------------------------------------------------------

struct SharedEmbedder(Arc<dyn Embedder>);
struct SharedParser(Arc<dyn QueryParser>);
struct SharedReranker(Arc<dyn Reranker>);

impl Embedder for SharedEmbedder {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn dims(&self) -> usize {
        self.0.dims()
    }
    fn is_remote(&self) -> bool {
        self.0.is_remote()
    }
    fn embed(
        &self,
        texts: &[&str],
        kind: EmbedKind,
    ) -> openstance_core::model::Result<Vec<Vec<f32>>> {
        self.0.embed(texts, kind)
    }
}

impl QueryParser for SharedParser {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn is_remote(&self) -> bool {
        self.0.is_remote()
    }
    fn parse(&self, text: &str, today: i64) -> openstance_core::model::Result<Query> {
        self.0.parse(text, today)
    }
}

impl Reranker for SharedReranker {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn is_remote(&self) -> bool {
        self.0.is_remote()
    }
    fn rerank(
        &self,
        q: &Query,
        cands: Vec<Candidate>,
    ) -> openstance_core::model::Result<Vec<Ranked>> {
        self.0.rerank(q, cands)
    }
}

/// `Models` whose three seams all delegate to the same shared instances.
fn shared_models(e: &Arc<dyn Embedder>, p: &Arc<dyn QueryParser>, r: &Arc<dyn Reranker>) -> Models {
    Models {
        embedder: Box::new(SharedEmbedder(Arc::clone(e))),
        parser: Box::new(SharedParser(Arc::clone(p))),
        reranker: Box::new(SharedReranker(Arc::clone(r))),
    }
}

fn reader_providers(models: Models) -> Providers {
    Providers {
        importers: Vec::new(),
        models,
        enricher: Box::new(NullEnricher),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openstance_core::FixedClock;
    use std::time::Duration;

    fn clock() -> ClockFn {
        Arc::new(|| Box::new(FixedClock(1_000)) as Box<dyn Clock>)
    }

    fn pool(readers: usize) -> (tempfile::TempDir, GraphPool) {
        let dir = tempfile::tempdir().unwrap();
        let p = GraphPool::open(
            Config::default(),
            &dir.path().join("graph.db"),
            Providers::null(),
            clock(),
            readers,
        )
        .unwrap();
        (dir, p)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn readers_see_committed_writes() {
        let (_d, p) = pool(2);
        p.write(|g| {
            g.write(|tx, _| {
                tx.execute(
                    "INSERT INTO source(kind, imported_at) VALUES ('vcf', 1)",
                    [],
                )?;
                Ok(())
            })
            .map_err(ApiError::from)
        })
        .await
        .unwrap();
        let n = p
            .read(|g| Ok(g.counts().map_err(ApiError::from)?.sources))
            .await
            .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_readers_run_at_the_same_time() {
        let (_d, p) = pool(2);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let run = |b: Arc<std::sync::Barrier>| {
            let p = p.clone();
            async move {
                p.read(move |_| {
                    // Both calls must be inside a reader at once, or this
                    // waits forever (the test then times out).
                    b.wait();
                    Ok(())
                })
                .await
            }
        };
        let both = futures_join(run(Arc::clone(&barrier)), run(Arc::clone(&barrier)));
        tokio::time::timeout(Duration::from_secs(10), both)
            .await
            .expect("readers ran concurrently");
    }

    async fn futures_join<A, B>(a: A, b: B)
    where
        A: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
        B: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        let (x, y) = (tokio::spawn(a), tokio::spawn(b));
        x.await.unwrap().unwrap();
        y.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_panicking_call_returns_its_reader_and_reports_internal() {
        let (_d, p) = pool(1);
        let err = p
            .read(|_| -> Result<(), ApiError> { panic!("boom") })
            .await
            .unwrap_err();
        assert_eq!(err.body["error"]["code"], "internal");
        // The only reader came back.
        tokio::time::timeout(Duration::from_secs(10), p.read(|_| Ok(())))
            .await
            .expect("reader returned")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn errors_pass_through() {
        let (_d, p) = pool(1);
        let err = p
            .write(|_| -> Result<(), ApiError> { Err(ApiError::not_found("x")) })
            .await
            .unwrap_err();
        assert_eq!(err.status.as_u16(), 404);
    }

    #[test]
    fn shared_models_delegate_to_one_instance() {
        let m = Models::null();
        let (e, pa, r): (Arc<dyn Embedder>, Arc<dyn QueryParser>, Arc<dyn Reranker>) = (
            Arc::from(m.embedder),
            Arc::from(m.parser),
            Arc::from(m.reranker),
        );
        let a = shared_models(&e, &pa, &r);
        assert_eq!(a.embedder.id(), e.id());
        assert_eq!(a.parser.id(), pa.id());
        assert_eq!(a.reranker.id(), r.id());
        assert_eq!(Arc::strong_count(&e), 2);
    }
}

use entmoot_query::{KnowledgeRead, RepositoryReader};
use entmoot_query::{
    KnowledgeRepository, KnowledgeStore, QueryEngine, SqliteKnowledgeRepository, TopologyGraph,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct ObservedReader {
    inner: entmoot_query::read::SqliteReader,
    opens: AtomicUsize,
}
impl RepositoryReader for ObservedReader {
    fn open_read(&self) -> Result<Box<dyn KnowledgeRead>, entmoot_query::KnowledgeRepositoryError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.inner.open_read()
    }
    fn exists(&self) -> bool {
        self.inner.exists()
    }
}

#[test]
fn query_opens_the_supplied_read_capability_instead_of_its_legacy_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("actual.sqlite3");
    let graph = TopologyGraph::new("fixture/read-port");
    SqliteKnowledgeRepository::open(&path)
        .unwrap()
        .replace_all(&KnowledgeStore::extract_from_graph(&graph))
        .unwrap();
    let reader = Arc::new(ObservedReader {
        inner: entmoot_query::read::SqliteReader::new(&path),
        opens: AtomicUsize::new(0),
    });
    let absent = directory.path().join("must-not-be-opened.sqlite3");
    let query = QueryEngine::with_backend(
        graph.clone(),
        &absent,
        Default::default(),
        reader.clone(),
        None,
    );
    assert_eq!(query.complete_graph().unwrap().snapshot, graph.snapshot);
    assert_eq!(reader.opens.load(Ordering::SeqCst), 1);
    assert!(!absent.exists());
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use entmoot_contracts::GitTreeCapture;
    use entmoot_ops::checkpoints::git::RetainedGitCapture;
    use entmoot_ops::*;
    use std::collections::{BTreeMap, BTreeSet};
    #[test]
    fn captures_share_bytes_and_survive_collection_until_their_pinned_view_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("app.py"), "value = 1\n").unwrap();
        let graph = entmoot_ops::build_topology_from_repo_with_options(
            &source,
            entmoot_ops::TopologyBuildOptions {
                repository: Some("gc/git"),
                ..Default::default()
            },
        )
        .unwrap();
        let store = dir.path().join("shared.db");
        let row = entmoot_ops::collaboration::register_graph(&store, &graph, "w", "agent", None, 0)
            .unwrap();
        let bytes = b"retained raw nonstructural input".to_vec();
        let digest = entmoot_contracts::blake3_hex(&bytes);
        let capture = RetainedGitCapture {
            receipt: GitTreeCapture {
                view: row.view.clone(),
                source: entmoot_ops::graph_source(&graph).unwrap(),
                parent_commit: "0".repeat(40),
                object_format: "sha1".into(),
                publisher: "agent".into(),
                files: vec![entmoot_contracts::GitTreeFile {
                    path: "raw.bin".into(),
                    mode: "100644".into(),
                    content_digest: ContentDigest::from_trusted(digest.clone()),
                }],
                limitations: vec![],
            },
            blobs: BTreeMap::from([(digest.clone(), bytes.clone())]),
        };
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        let row = db.publish_git_capture(row, &capture, 1).unwrap();
        let id = row.git_capture.as_ref().unwrap().to_string();
        let job = entmoot_ops::collaboration::submit_conflicts(
            &mut db,
            entmoot_contracts::ConflictBatchRequest {
                git_merge: true,
                base_view: row.view.clone(),
                left: row.clone(),
                peers: vec![],
            },
            "gc",
            "agent",
        )
        .unwrap();
        db.retire_workspace("w", 2).unwrap();
        drop(db);
        let remove = BTreeSet::from([row.view.to_string()]);
        let mut maintenance =
            entmoot_ops::knowledge_db::retention::Maintenance::open(&store).unwrap();
        maintenance.collect(&BTreeSet::new()).unwrap();
        assert!(maintenance.collect(&remove).is_err());
        drop(maintenance);
        let mut db = SqliteKnowledgeRepository::open(&store).unwrap();
        assert_eq!(db.git_capture(&id).unwrap().blobs[&digest], bytes);
        db.control_operation(job.id.as_str(), entmoot_contracts::OperationControl::Cancel)
            .unwrap();
        drop(db);
        {
            let mut maintenance =
                entmoot_ops::knowledge_db::retention::Maintenance::open(&store).unwrap();
            maintenance.collect(&remove).unwrap();
        }
        let db = SqliteKnowledgeRepository::open(&store).unwrap();
        assert!(db.git_capture(&id).is_err());
        assert_eq!(
            rusqlite::Connection::open(&store)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM local_git_files", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            rusqlite::Connection::open(&store)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM shared_source_files WHERE digest=?1",
                    [digest],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
}

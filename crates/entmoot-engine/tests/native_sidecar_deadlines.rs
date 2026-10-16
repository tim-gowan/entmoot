//! Host transport controls shared by every external native adapter.
use entmoot_contracts::CapturedOutput;
use entmoot_engine::native_resolver::{run_sidecar, SidecarRequest};
use std::time::{Duration, Instant};

#[allow(clippy::unwrap_used)]
fn run(
    script: &str,
    flags: Vec<String>,
    wall: Duration,
) -> (
    entmoot_engine::native_resolver::SidecarResponse,
    Duration,
    tempfile::TempDir,
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("worker.py");
    std::fs::write(&path, script).unwrap();
    let started = Instant::now();
    let response = run_sidecar(
        &path,
        &SidecarRequest {
            canon: false,
            root: root.path().to_string_lossy().into(),
            patterns: vec![],
            flags,
            plan: false,
        },
        wall,
    )
    .unwrap();
    (response, started.elapsed(), root)
}

#[test]
fn deadline_covers_descendant_output_handles_and_stdin_backpressure() {
    for read_stdin in [true, false] {
        let script = format!("import sys, subprocess, time\n{}\nsubprocess.Popen([sys.executable, '-c', 'import time; time.sleep(10)'])\nsys.stdout.buffer.write(b'partial\\xff'); sys.stdout.flush()\nsys.stderr.buffer.write(b'error\\x80'); sys.stderr.flush()\ntime.sleep(10)\n",
            if read_stdin { "sys.stdin.read()" } else { "pass" });
        let (response, elapsed, _) = run(
            &script,
            vec!["x".repeat(1024 * 1024)],
            // The budget includes cold interpreter startup, reading the large
            // request, and spawning the descendant. Give a loaded test host time
            // to reach the blocked work whose output/cleanup we are asserting.
            // Both processes still sleep for ten seconds, beyond this deadline.
            Duration::from_secs(2),
        );
        assert!(response.budget_exceeded, "{response:?}");
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
        let output = response.native_output.unwrap();
        assert_eq!(
            output.stdout,
            CapturedOutput::Bytes(b"partial\xff".to_vec())
        );
        assert_eq!(output.stderr, CapturedOutput::Bytes(b"error\x80".to_vec()));
    }
}

#[test]
fn exited_parent_cannot_leave_a_writer_running_or_block_capture() {
    let script = r#"import sys, subprocess, json
request = json.load(sys.stdin)
child = "import time,pathlib; time.sleep(1.5); pathlib.Path(" + repr(request['root'] + '/leaked') + ").write_text('alive')"
subprocess.Popen([sys.executable, '-c', child])
print('{}', flush=True)
"#;
    let wall = Duration::from_secs(3);
    let (response, elapsed, root) = run(script, vec![], wall);
    assert!(!response.budget_exceeded && !response.invalid_environment);
    // The requested budget includes cold interpreter startup. The delayed
    // write below independently detects a child surviving parent cleanup.
    assert!(elapsed < wall, "{elapsed:?}");
    std::thread::sleep(Duration::from_millis(1700));
    assert!(
        !root.path().join("leaked").exists(),
        "descendant escaped cleanup"
    );
}

#[test]
fn large_streams_are_lossless_without_pipe_deadlock() {
    let (response, _, _) = run("import sys,json\njson.load(sys.stdin)\nsys.stderr.buffer.write(b'e'*1048576)\nprint(json.dumps({'diagnostics':['x'*1048576]}))\n", vec![], Duration::from_secs(10));
    assert!(!response.budget_exceeded && !response.invalid_environment);
    assert_eq!(response.diagnostics, vec!["x".repeat(1024 * 1024)]);
    assert_eq!(
        response.native_output.unwrap().stderr,
        CapturedOutput::Utf8("e".repeat(1024 * 1024))
    );
}

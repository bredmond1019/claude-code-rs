//! Process-group teardown: a timed-out or dropped call kills the CLI *and* its background
//! children. The child is a fake `/bin/sh` script named by `CLAUDE_BINARY`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use claude_sdk_rs::{Config, Error};

/// `CLAUDE_BINARY` is process-global; every test that sets it holds this async lock.
static BINARY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn write_fake_binary(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-claude.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn wait_gone(pid: i32, within: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < within {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    !alive(pid)
}

/// Waits for the fake to write its pid file (`<background pid> <self pid>`) and returns both.
async fn read_pids(file: &Path) -> (i32, i32) {
    for _ in 0..100 {
        if let Ok(s) = std::fs::read_to_string(file) {
            let v: Vec<i32> = s
                .split_whitespace()
                .filter_map(|p| p.parse().ok())
                .collect();
            if v.len() == 2 {
                return (v[0], v[1]);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fake never recorded its pids");
}

fn spawning_body(pids: &Path, prefix: &str) -> String {
    format!(
        "{prefix}sleep 600 &\necho \"$! $$\" > {}\nexec sleep 600\n",
        pids.display()
    )
}

#[tokio::test]
async fn timeout_kills_the_cli_and_its_background_child() {
    let _lock = BINARY_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let pids = dir.path().join("pids");
    let binary = write_fake_binary(dir.path(), &spawning_body(&pids, ""));
    unsafe { std::env::set_var("CLAUDE_BINARY", &binary) };

    let config = Config {
        timeout: Some(Duration::from_millis(1500)),
        ..Config::default()
    };
    let call = tokio::spawn(async move { claude_sdk_rs::execute(&config, "hi").await });
    let (bg, cli) = read_pids(&pids).await;
    assert!(alive(bg) && alive(cli), "both alive before the deadline");

    let result = call.await.unwrap();
    assert!(matches!(result, Err(Error::Timeout)), "got {result:?}");
    assert!(
        wait_gone(bg, Duration::from_secs(5)).await,
        "background child gone"
    );
    assert!(wait_gone(cli, Duration::from_secs(5)).await, "cli gone");
    unsafe { std::env::remove_var("CLAUDE_BINARY") };
}

#[tokio::test]
async fn dropping_the_call_future_kills_the_whole_group() {
    let _lock = BINARY_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let pids = dir.path().join("pids");
    let binary = write_fake_binary(dir.path(), &spawning_body(&pids, ""));
    unsafe { std::env::set_var("CLAUDE_BINARY", &binary) };

    let config = Config::default();
    let outer = tokio::time::timeout(
        Duration::from_millis(1500),
        claude_sdk_rs::execute(&config, "hi"),
    );
    let pids_file = pids.clone();
    let probe = async move {
        let (bg, cli) = read_pids(&pids_file).await;
        assert!(alive(bg) && alive(cli));
        (bg, cli)
    };
    let (outer_result, (bg, cli)) = tokio::join!(outer, probe);
    assert!(outer_result.is_err(), "wrapper elapsed, future dropped");
    assert!(
        wait_gone(bg, Duration::from_secs(5)).await,
        "background child gone"
    );
    assert!(wait_gone(cli, Duration::from_secs(5)).await, "cli gone");
    unsafe { std::env::remove_var("CLAUDE_BINARY") };
}

#[tokio::test]
async fn sigterm_ignoring_group_is_killed_only_after_the_configured_grace() {
    let _lock = BINARY_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let pids = dir.path().join("pids");
    // SIGTERM is ignored by the whole group (disposition inherited across exec).
    let binary = write_fake_binary(dir.path(), &spawning_body(&pids, "trap '' TERM\n"));
    unsafe { std::env::set_var("CLAUDE_BINARY", &binary) };

    let grace = Duration::from_secs(3);
    let config = Config {
        timeout: Some(Duration::from_millis(1000)),
        group_kill_grace: Some(grace),
        ..Config::default()
    };
    let started = Instant::now();
    let call = tokio::spawn(async move { claude_sdk_rs::execute(&config, "hi").await });
    let (bg, _cli) = read_pids(&pids).await;
    let result = call.await.unwrap();
    assert!(matches!(result, Err(Error::Timeout)), "got {result:?}");
    let dropped_at = Instant::now();

    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(
        alive(bg),
        "SIGTERM-ignoring child still alive inside the grace"
    );

    assert!(
        wait_gone(bg, Duration::from_secs(5)).await,
        "SIGKILL follows"
    );
    assert!(
        dropped_at.elapsed() >= grace - Duration::from_millis(300),
        "killed before the grace elapsed: {:?} (call took {:?})",
        dropped_at.elapsed(),
        started.elapsed()
    );
    unsafe { std::env::remove_var("CLAUDE_BINARY") };
}

#[tokio::test]
async fn normal_exit_yields_the_same_outcome() {
    let _lock = BINARY_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let binary = write_fake_binary(
        dir.path(),
        "printf '{\"total_cost_usd\":0.0,\"usage\":{},\"is_error\":false,\"result\":\"ok\"}'\n",
    );
    unsafe { std::env::set_var("CLAUDE_BINARY", &binary) };

    let outcome = claude_sdk_rs::execute(&Config::default(), "hi")
        .await
        .expect("normal exit");
    assert_eq!(outcome.text, "ok");
    unsafe { std::env::remove_var("CLAUDE_BINARY") };
}

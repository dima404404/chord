use std::{env, path::Path, process::Command};

use chord_client::data::{self, DataError};
use serde_json::json;

const FIXTURE_TEST_ROOT: &str = "CHORD_TEST_PORTABLE_FIXTURE_ROOT";

#[test]
fn portable_session_loads_without_developer_fixtures() {
    if let Some(root) = env::var_os(FIXTURE_TEST_ROOT) {
        check_portable_session(Path::new(&root));
        return;
    }

    // Isolate DL_FIXTURES and the process-wide session state from the other contracts.
    let package = tempfile::tempdir().expect("create a clean portable package directory");
    let root = package.path().join("fixtures");
    let output = Command::new(env::current_exe().expect("locate the contract test binary"))
        .args([
            "--exact",
            "portable_session_loads_without_developer_fixtures",
            "--nocapture",
        ])
        .current_dir(package.path())
        .env(FIXTURE_TEST_ROOT, &root)
        .env("DL_FIXTURES", &root)
        .output()
        .expect("check an isolated portable session");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_portable_session(root: &Path) {
    assert!(!root.exists());
    assert!(matches!(
        data::fixtures_root(),
        Err(DataError::NotFound { resource, .. }) if resource == "fixtures"
    ));

    let user = json!({ "id": "777000", "username": "portable_fixture" });
    data::set_live_components_sqlite_path(Some(root.with_file_name("test.sqlite3")));
    data::set_live_session(Some("live-test-token".to_owned()), None);
    data::ingest_live_dispatch(
        "READY",
        &json!({
            "session_id": "sess_portable",
            "user": user,
            "guilds": [{
                "id": "888000",
                "properties": { "name": "Portable guild" },
                "member_count": 1,
                "channels": [{
                    "id": "888001",
                    "type": 0,
                    "name": "general",
                    "position": 1
                }],
                "members": [{ "user": user, "roles": [] }]
            }],
            "read_state": { "entries": [] }
        }),
    );

    let resolved = data::fixtures_root().expect("live data must not require developer fixtures");
    assert_eq!(resolved, root);
    let ready = data::ready(&resolved).expect("load the server list");
    assert_eq!(ready.guilds.len(), 1);
    assert_eq!(ready.guilds[0].name, "Portable guild");

    let channels = data::channels(&resolved, Some("888000")).expect("load guild channels");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].id, "888001");

    let members = data::members_page(&resolved, "888000", None, None).expect("load guild members");
    assert!(members.items.iter().any(|member| member.id == "777000"));
    data::record_sent_message(
        "888001",
        json!({
            "id": "5002",
            "channel_id": "888001",
            "content": "A message already cached in this session",
            "timestamp": "2026-11-01T12:00:00+00:00",
            "author": user
        }),
    );
    let messages = data::messages_page(&resolved, "888001", None, None).expect("load chat");
    assert!(messages.items.iter().any(|message| message.id == "5002"));
    assert!(!root.exists());
}

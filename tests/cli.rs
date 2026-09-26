use markframe::{
    api::{AppState, router},
    storage::Storage,
};
use serde_json::{Value, json};
use std::{io::Cursor, process::Output};
use tokio::process::Command;

async fn cli(server: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_markframe"))
        .args(args)
        .args(["--server", server])
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap()
}
fn result(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[tokio::test]
async fn management_cli_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path().join("data")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async {
        axum::serve(listener, router(AppState::new(storage)))
            .await
            .unwrap()
    });
    let original = dir.path().join("original.png");
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 4)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    std::fs::write(&original, bytes.into_inner()).unwrap();
    let first = result(cli(&url, &["post", original.to_str().unwrap()]).await);
    let id = first["id"].as_str().unwrap();
    let list = result(cli(&url, &["list", "--json"]).await);
    assert_eq!(list["images"][0]["id"], id);
    assert_eq!(list["images"][0]["annotation_count"], 0);
    assert!(list["images"][0]["size_bytes"].as_u64().unwrap() > 0);
    let table = cli(&url, &["list"]).await;
    assert!(String::from_utf8_lossy(&table.stdout).contains("original.png"));
    for args in [
        vec!["clear", "--json"],
        vec!["delete", id, "--json"],
        vec!["prune", "--keep-last", "0"],
    ] {
        let output = cli(&url, &args).await;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--yes"));
    }
    assert_eq!(
        result(cli(&url, &["list", "--json"]).await)["images"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let preview = result(cli(&url, &["clear", "--dry-run", "--json"]).await);
    assert_eq!(preview["target_ids"], json!([id]));
    assert_eq!(preview["deleted_ids"], json!([]));
    assert!(!cli(&url, &["prune", "--yes"]).await.status.success());
    assert!(
        !cli(&url, &["prune", "--older-than", "0d", "--yes"])
            .await
            .status
            .success()
    );
    assert_eq!(
        result(cli(&url, &["prune", "--older-than", "7d", "--yes", "--json"]).await)["target_ids"],
        json!([])
    );
    let second = result(cli(&url, &["post", original.to_str().unwrap()]).await);
    let preview = result(cli(&url, &["prune", "--keep-last", "1", "--dry-run", "--json"]).await);
    assert_eq!(preview["target_ids"], json!([id]));
    let absent = uuid::Uuid::new_v4().to_string();
    let output = cli(&url, &["delete", id, &absent, id, "--yes", "--json"]).await;
    assert!(!output.status.success());
    let partial: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(partial["deleted_ids"], json!([id]));
    assert_eq!(partial["missing_ids"], json!([absent]));
    assert_eq!(
        result(cli(&url, &["clear", "--yes", "--json"]).await)["deleted_ids"],
        json!([second["id"]])
    );
    assert_eq!(
        result(cli(&url, &["clear", "--json"]).await)["deleted_ids"],
        json!([])
    );
    assert!(original.is_file());
    server.abort();
}

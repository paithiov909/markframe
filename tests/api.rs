use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use image::{DynamicImage, ImageFormat};
use markframe::{
    api::{AppState, IMAGE_LIMIT, router},
    storage::Storage,
};
use serde_json::{Value, json};
use std::io::Cursor;
use tower::ServiceExt;
fn setup() -> (tempfile::TempDir, AppState, Router) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(Storage::open(dir.path().into()).unwrap());
    let app = router(state.clone());
    (dir, state, app)
}
fn image(format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    DynamicImage::new_rgb8(100, 80)
        .write_to(&mut out, format)
        .unwrap();
    out.into_inner()
}
async fn request(
    app: &Router,
    method: &str,
    url: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(url);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let res = app
        .clone()
        .oneshot(
            req.body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
async fn upload(app: &Router, bytes: Vec<u8>, filename: &str) -> (StatusCode, Value) {
    let mut body=format!("--test\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{filename}\"\r\nContent-Type: image/png\r\n\r\n").into_bytes();
    body.extend(bytes);
    body.extend(b"\r\n--test--\r\n");
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/images")
                .header("content-type", "multipart/form-data; boundary=test")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn annotation() -> Value {
    json!({"schema":"annotorious-v3","annotation":{"id":"test-annotation","bodies":[{"id":"body-1","annotation":"test-annotation","purpose":"commenting","value":"  この円を少し大きく\n "}],"target":{"annotation":"test-annotation","selector":{"type":"RECTANGLE","geometry":{"x":10,"y":10,"w":40,"h":30,"bounds":{"minX":10,"minY":10,"maxX":50,"maxY":40}}}}}})
}
#[tokio::test]
async fn image_formats_current_history_and_original() {
    let (_dir, _state, app) = setup();
    assert_eq!(
        request(&app, "GET", "/api/current", None).await.1,
        json!({"image":null})
    );
    let mut ids = vec![];
    for (format, mime) in [
        (ImageFormat::Png, "image/png"),
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let bytes = image(format);
        let (status, meta) = upload(&app, bytes.clone(), "../../sample.img").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(meta["filename"], "sample.img");
        assert_eq!(meta["mime_type"], mime);
        assert_eq!(meta["width"], 100);
        assert_eq!(meta["height"], 80);
        let id = meta["id"].as_str().unwrap().to_owned();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/images/{id}/content"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.headers()["content-type"], mime);
        assert_eq!(res.into_body().collect().await.unwrap().to_bytes(), bytes);
        ids.push(id);
    }
    assert_eq!(
        request(&app, "GET", "/api/current", None).await.1["image"]["id"],
        ids[2]
    );
    let list = request(&app, "GET", "/api/images", None).await.1;
    assert_eq!(list["images"].as_array().unwrap().len(), 3);
    assert_eq!(list["images"][2]["id"], ids[0]);
}
#[tokio::test]
async fn crud_preview_and_persistence() {
    let (dir, state, app) = setup();
    let meta = upload(&app, image(ImageFormat::Png), "sample.png").await.1;
    let id = meta["id"].as_str().unwrap();
    let url = format!("/api/images/{id}/annotations");
    let one = format!("{url}/test-annotation");
    let envelope = annotation();
    assert_eq!(
        request(&app, "POST", &url, Some(envelope.clone())).await,
        (StatusCode::CREATED, envelope.clone())
    );
    assert_eq!(
        request(&app, "POST", &url, Some(envelope.clone())).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, "GET", &one, None).await.1["annotation"],
        envelope["annotation"]
    );
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{one}/preview"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let preview = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(preview.dimensions(), (100, 80));
    assert_ne!(preview.get_pixel(11, 11), preview.get_pixel(60, 60));
    let mut edited = envelope.clone();
    // Annotorious can retain the previous drag frame in cached bounds.
    edited["annotation"]["target"]["selector"]["geometry"]["x"] = json!(15);
    edited["annotation"]["bodies"][0]["value"] = json!("\n exact text  ");
    assert_eq!(
        request(&app, "PUT", &one, Some(edited.clone())).await.0,
        StatusCode::OK
    );
    drop(app);
    drop(state);
    let reopened = AppState::new(Storage::open(dir.path().into()).unwrap());
    let app = router(reopened);
    assert_eq!(
        request(&app, "GET", "/api/current", None).await.1["image"]["id"],
        id
    );
    assert_eq!(
        request(&app, "GET", &url, None).await.1["annotations"][0],
        edited
    );
    assert_eq!(
        request(&app, "DELETE", &one, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "GET", &one, None).await.0,
        StatusCode::NOT_FOUND
    );
}
#[tokio::test]
async fn rejects_invalid_requests() {
    let (_dir, _state, app) = setup();
    for bytes in [
        b"text".to_vec(),
        b"\x89PNG\r\n\x1a\ninvalid".to_vec(),
        b"GIF89a12345".to_vec(),
    ] {
        assert!(upload(&app, bytes, "bad.png").await.0.is_client_error());
    }
    assert_eq!(
        upload(&app, vec![0; IMAGE_LIMIT + 1], "big.png").await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        request(&app, "GET", "/api/images/not-an-id/annotations", None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("/api/images/{}/annotations", uuid::Uuid::new_v4()),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let meta = upload(&app, image(ImageFormat::Png), "a.png").await.1;
    let url = format!("/api/images/{}/annotations", meta["id"].as_str().unwrap());
    let mut bad = annotation();
    bad["annotation"]["target"]["selector"]["geometry"]["w"] = json!(1000);
    assert_eq!(
        request(&app, "POST", &url, Some(bad)).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "POST", &url, Some(json!({}))).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(&app, "POST", "/api/health", None).await.0,
        StatusCode::METHOD_NOT_ALLOWED
    );
}
#[tokio::test]
async fn sse_announces_committed_image() {
    let (_dir, _state, app) = setup();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.headers()["content-type"], "text/event-stream");
    let mut body = res.into_body();
    let meta = upload(&app, image(ImageFormat::Png), "a.png").await.1;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = std::str::from_utf8(frame.data_ref().unwrap()).unwrap();
    assert!(text.contains("event: image"));
    assert!(text.contains(meta["id"].as_str().unwrap()));
}
#[test]
fn storage_lock_and_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let store = Storage::open(dir.path().into()).unwrap();
    assert!(Storage::open(dir.path().into()).is_err());
    drop(store);
    std::fs::write(dir.path().join("state.json"), "broken").unwrap();
    assert!(Storage::open(dir.path().into()).is_err());
}

#[tokio::test]
async fn failed_commit_keeps_previous_state_and_emits_no_event() {
    let (dir, state, app) = setup();
    let first = upload(&app, image(ImageFormat::Png), "first.png").await.1;
    let mut events = state.events.subscribe();
    std::fs::create_dir(dir.path().join("state.json.tmp")).unwrap();
    assert_eq!(
        upload(&app, image(ImageFormat::Png), "second.png").await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        request(&app, "GET", "/api/current", None).await.1["image"],
        first
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("images"))
            .unwrap()
            .count(),
        1
    );
    assert!(events.try_recv().is_err());
    let url = format!("/api/images/{}/annotations", first["id"].as_str().unwrap());
    assert_eq!(
        request(&app, "POST", &url, Some(annotation())).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        request(&app, "GET", &url, None).await.1["annotations"],
        json!([])
    );
}

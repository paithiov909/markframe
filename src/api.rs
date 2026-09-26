use crate::storage::{Envelope, ImageMeta, Storage};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Multipart, Path, State, multipart::MultipartRejection,
        rejection::JsonRejection,
    },
    http::{StatusCode, header},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use futures_util::StreamExt;
use image::{ImageFormat, ImageReader, Rgba};
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    io::Cursor,
    sync::{Arc, Mutex},
};
use tokio::sync::{Semaphore, broadcast};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

pub const IMAGE_LIMIT: usize = 25 * 1024 * 1024;
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Mutex<Storage>>,
    pub events: broadcast::Sender<String>,
    work: Arc<Semaphore>,
}
impl AppState {
    pub fn new(storage: Storage) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            storage: Arc::new(Mutex::new(storage)),
            events,
            work: Arc::new(Semaphore::new(2)),
        }
    }
}
#[derive(Debug)]
pub struct ApiError(StatusCode, &'static str, String);
type Result<T> = std::result::Result<T, ApiError>;
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({"error":{"code":self.1,"message":self.2}})),
        )
            .into_response()
    }
}
fn bad(message: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "invalid_request", message.into())
}
fn missing() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "not_found",
        "Image or annotation not found".into(),
    )
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    tracing::error!(error=%e, "operation failed");
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage_error",
        "Unable to complete operation".into(),
    )
}
fn id(value: &str) -> Result<()> {
    if Uuid::parse_str(value).is_err() {
        return Err(bad("Invalid image ID"));
    }
    Ok(())
}
fn annotation_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
    {
        return Err(bad("Invalid annotation ID"));
    }
    Ok(())
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(internal)?
}
fn decode(bytes: &[u8]) -> Result<(image::DynamicImage, &'static str)> {
    let format = image::guess_format(bytes).map_err(|_| bad("Invalid image data"))?;
    let mime = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        _ => {
            return Err(ApiError(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_image_type",
                "Supported formats: PNG, JPEG, WebP".into(),
            ));
        }
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| bad("Invalid image or decoded image exceeds limits"))?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > 32_000_000 {
        return Err(bad("Image exceeds 32 megapixels"));
    }
    Ok((decoded, mime))
}
#[derive(rust_embed::RustEmbed)]
#[folder = "frontend/dist/"]
struct Assets;
pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({"status":"ok"})) }),
        )
        .route("/api/images", get(list).post(upload))
        .route("/api/current", get(current))
        .route("/api/events", get(events))
        .route("/api/images/{image_id}/content", get(content))
        .route(
            "/api/images/{image_id}/annotations",
            get(annotations).post(create),
        )
        .route(
            "/api/images/{image_id}/annotations/{annotation_id}",
            get(one).put(update).delete(remove),
        )
        .route(
            "/api/images/{image_id}/annotations/{annotation_id}/preview",
            get(preview),
        )
        .route("/api", get(|| async { missing() }))
        .route("/api/{*rest}", get(|| async { missing() }))
        .fallback(static_file)
        .method_not_allowed_fallback(|| async {
            ApiError(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "Method not allowed".into(),
            )
        })
        .layer(DefaultBodyLimit::max(IMAGE_LIMIT + 64 * 1024))
        .with_state(state)
}
async fn static_file(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(file) => (
            [
                (
                    header::CONTENT_TYPE,
                    mime_guess::from_path(path)
                        .first_or_octet_stream()
                        .to_string(),
                ),
                (header::CACHE_CONTROL, "no-cache".into()),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => missing().into_response(),
    }
}
async fn list(State(s): State<AppState>) -> Result<Json<Value>> {
    blocking(move || {
        Ok(Json(
            json!({"images":s.storage.lock().map_err(internal)?.data.images}),
        ))
    })
    .await
}
async fn current(State(s): State<AppState>) -> Result<Json<Value>> {
    blocking(move || {
        Ok(Json(
            json!({"image":s.storage.lock().map_err(internal)?.data.images.first()}),
        ))
    })
    .await
}
fn multipart_error(e: axum::extract::multipart::MultipartError) -> ApiError {
    ApiError(e.status(), "invalid_upload", e.body_text())
}
async fn upload(
    State(s): State<AppState>,
    multipart: std::result::Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<ImageMeta>)> {
    let _permit = s.work.clone().acquire_owned().await.map_err(internal)?;
    let mut multipart = multipart.map_err(|e| bad(&e.body_text()))?;
    let mut image = None;
    let mut name = None;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("image") => {
                if image.is_some() {
                    return Err(bad("Only one image field is allowed"));
                }
                let filename = field
                    .file_name()
                    .unwrap_or("image")
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("image")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(255)
                    .collect::<String>();
                let mut bytes = Vec::new();
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    if bytes.len() + chunk.len() > IMAGE_LIMIT {
                        return Err(ApiError(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "upload_too_large",
                            "Image exceeds 25 MiB".into(),
                        ));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                image = Some((
                    if filename.is_empty() {
                        "image".into()
                    } else {
                        filename
                    },
                    bytes,
                ));
            }
            Some("name") => {
                let value = field.text().await.map_err(multipart_error)?;
                if value.len() > 4096 {
                    return Err(bad("Name is too long"));
                }
                name = Some(value);
            }
            _ => return Err(bad("Expected image and optional name fields")),
        }
    }
    let (filename, bytes) = image.ok_or_else(|| bad("Missing image field"))?;
    blocking(move || {
        let (decoded, mime) = decode(&bytes)?;
        let meta = ImageMeta {
            id: Uuid::new_v4().to_string(),
            filename,
            name,
            mime_type: mime.into(),
            width: decoded.width(),
            height: decoded.height(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let mut store = s.storage.lock().map_err(internal)?;
        let path = store.root.join("images").join(&meta.id);
        std::fs::write(&path, bytes).map_err(internal)?;
        let mut next = store.data.clone();
        next.images.insert(0, meta.clone());
        next.annotations.insert(meta.id.clone(), vec![]);
        if let Err(e) = store.commit(next) {
            let _ = std::fs::remove_file(path);
            return Err(internal(e));
        }
        let _ = s.events.send(meta.id.clone());
        Ok((StatusCode::CREATED, Json(meta)))
    })
    .await
}
async fn content(State(s): State<AppState>, Path(image_id): Path<String>) -> Result<Response> {
    id(&image_id)?;
    blocking(move || {
        let store = s.storage.lock().map_err(internal)?;
        let meta = store
            .data
            .images
            .iter()
            .find(|i| i.id == image_id)
            .ok_or_else(missing)?;
        let bytes = std::fs::read(store.root.join("images").join(&image_id)).map_err(internal)?;
        Ok((
            [
                (header::CONTENT_TYPE, meta.mime_type.clone()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            ],
            bytes,
        )
            .into_response())
    })
    .await
}
async fn annotations(
    State(s): State<AppState>,
    Path(image_id): Path<String>,
) -> Result<Json<Value>> {
    id(&image_id)?;
    blocking(move || {
        let store = s.storage.lock().map_err(internal)?;
        let anns = store.data.annotations.get(&image_id).ok_or_else(missing)?;
        Ok(Json(json!({"image_id":image_id,"annotations":anns})))
    })
    .await
}
fn rect(a: &Value) -> Result<[f64; 4]> {
    if a.pointer("/target/selector/type").and_then(Value::as_str) != Some("RECTANGLE") {
        return Err(bad("Only RECTANGLE selectors are supported"));
    }
    let g = &a["target"]["selector"]["geometry"];
    let mut values = [0.0; 4];
    for (n, key) in ["x", "y", "w", "h"].iter().enumerate() {
        values[n] = g[key]
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| bad("Invalid rectangle geometry"))?;
    }
    if values[0] < 0.0 || values[1] < 0.0 || values[2] <= 0.0 || values[3] <= 0.0 {
        return Err(bad("Invalid rectangle dimensions"));
    }
    Ok(values)
}
fn validate(e: &Envelope, meta: &ImageMeta) -> Result<String> {
    if e.schema != "annotorious-v3" {
        return Err(bad("Unsupported annotation schema"));
    }
    let aid = e.annotation["id"]
        .as_str()
        .ok_or_else(|| bad("Missing annotation ID"))?;
    annotation_id(aid)?;
    if !e.annotation["bodies"].is_array() {
        return Err(bad("Annotation bodies must be an array"));
    }
    if e.annotation
        .pointer("/target/annotation")
        .and_then(Value::as_str)
        != Some(aid)
    {
        return Err(bad("Target annotation must match annotation ID"));
    }
    let [x, y, w, h] = rect(&e.annotation)?;
    if x + w > meta.width as f64 + 0.01 || y + h > meta.height as f64 + 0.01 {
        return Err(bad("Rectangle lies outside the source image"));
    }
    for body in e.annotation["bodies"].as_array().unwrap() {
        if body["id"].as_str().is_none()
            || body["annotation"].as_str() != Some(aid)
            || body.get("value").is_some_and(|v| !v.is_string())
        {
            return Err(bad("Invalid annotation body"));
        }
    }
    let geometry = &e.annotation["target"]["selector"]["geometry"];
    if geometry.get("rot").is_some_and(|v| v.as_f64() != Some(0.0)) {
        return Err(bad("Only axis-aligned rectangles are supported"));
    }
    // Annotorious may report cached bounds from the preceding drag frame.
    // Validate their shape, but use x/y/w/h as the authoritative rectangle.
    for key in ["minX", "minY", "maxX", "maxY"] {
        if !geometry["bounds"][key].as_f64().is_some_and(f64::is_finite) {
            return Err(bad("Rectangle bounds must contain finite coordinates"));
        }
    }
    Ok(aid.into())
}
async fn create(
    State(s): State<AppState>,
    Path(i): Path<String>,
    body: std::result::Result<Json<Envelope>, JsonRejection>,
) -> Result<(StatusCode, Json<Envelope>)> {
    let Json(e) = body.map_err(|e| ApiError(e.status(), "invalid_json", e.body_text()))?;
    save(s, i, None, e)
        .await
        .map(|e| (StatusCode::CREATED, Json(e)))
}
async fn update(
    State(s): State<AppState>,
    Path((i, a)): Path<(String, String)>,
    body: std::result::Result<Json<Envelope>, JsonRejection>,
) -> Result<Json<Envelope>> {
    annotation_id(&a)?;
    let Json(e) = body.map_err(|e| ApiError(e.status(), "invalid_json", e.body_text()))?;
    save(s, i, Some(a), e).await.map(Json)
}
async fn save(s: AppState, i: String, a: Option<String>, e: Envelope) -> Result<Envelope> {
    id(&i)?;
    blocking(move || {
        let mut store = s.storage.lock().map_err(internal)?;
        let meta = store
            .data
            .images
            .iter()
            .find(|m| m.id == i)
            .ok_or_else(missing)?;
        let aid = validate(&e, meta)?;
        let mut next = store.data.clone();
        let anns = next.annotations.get_mut(&i).ok_or_else(missing)?;
        if let Some(a) = a {
            if a != aid {
                return Err(bad("Annotation ID does not match URL"));
            }
            let found = anns
                .iter_mut()
                .find(|e| e.annotation["id"] == a)
                .ok_or_else(missing)?;
            *found = e.clone();
        } else {
            if anns.iter().any(|e| e.annotation["id"] == aid) {
                return Err(ApiError(
                    StatusCode::CONFLICT,
                    "annotation_exists",
                    "Annotation ID already exists".into(),
                ));
            }
            anns.push(e.clone());
        }
        store.commit(next).map_err(internal)?;
        Ok(e)
    })
    .await
}
async fn one(
    State(s): State<AppState>,
    Path((i, a)): Path<(String, String)>,
) -> Result<Json<Value>> {
    id(&i)?;
    annotation_id(&a)?;
    blocking(move || {
        let store = s.storage.lock().map_err(internal)?;
        let e = store
            .data
            .annotations
            .get(&i)
            .and_then(|anns| anns.iter().find(|e| e.annotation["id"] == a))
            .ok_or_else(missing)?;
        Ok(Json(
            json!({"image_id":i,"schema":e.schema,"annotation":e.annotation}),
        ))
    })
    .await
}
async fn remove(
    State(s): State<AppState>,
    Path((i, a)): Path<(String, String)>,
) -> Result<StatusCode> {
    id(&i)?;
    annotation_id(&a)?;
    blocking(move || {
        let mut store = s.storage.lock().map_err(internal)?;
        let mut next = store.data.clone();
        let anns = next.annotations.get_mut(&i).ok_or_else(missing)?;
        let pos = anns
            .iter()
            .position(|e| e.annotation["id"] == a)
            .ok_or_else(missing)?;
        anns.remove(pos);
        store.commit(next).map_err(internal)?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}
async fn preview(
    State(s): State<AppState>,
    Path((i, a)): Path<(String, String)>,
) -> Result<Response> {
    id(&i)?;
    annotation_id(&a)?;
    let _permit = s.work.clone().acquire_owned().await.map_err(internal)?;
    blocking(move || {
        let (bytes, geometry) = {
            let store = s.storage.lock().map_err(internal)?;
            let e = store
                .data
                .annotations
                .get(&i)
                .and_then(|anns| anns.iter().find(|e| e.annotation["id"] == a))
                .ok_or_else(missing)?;
            (
                std::fs::read(store.root.join("images").join(&i)).map_err(internal)?,
                rect(&e.annotation)?,
            )
        };
        let mut img = decode(&bytes)?.0.to_rgba8();
        let [x, y, w, h] = geometry;
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = ((x + w).ceil() as u32).min(img.width() - 1);
        let y1 = ((y + h).ceil() as u32).min(img.height() - 1);
        let thickness = (img.width().max(img.height()) / 400).clamp(3, 16);
        for py in y0..=y1 {
            for px in x0..=x1 {
                let d = (px - x0).min(x1 - px).min(py - y0).min(y1 - py);
                if d < thickness {
                    img.put_pixel(
                        px,
                        py,
                        if d == 0 {
                            Rgba([255, 255, 255, 255])
                        } else {
                            Rgba([255, 32, 72, 255])
                        },
                    );
                }
            }
        }
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).map_err(internal)?;
        Ok(([(header::CONTENT_TYPE, "image/png")], out.into_inner()).into_response())
    })
    .await
}
async fn events(
    State(s): State<AppState>,
) -> Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(s.events.subscribe()).map(|event| {
        Ok(Event::default().event("image").data(match event {
            Ok(id) => json!({"id":id}).to_string(),
            Err(_) => "{}".into(),
        }))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

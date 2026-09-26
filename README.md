# Markframe

A local visual review surface for AI-assisted coding. Post an image, inspect it in your browser, draw a rectangle, and copy your comment with machine-readable visual context.

> Markframe does not generate prompts. It preserves user-authored text and attaches machine-readable visual context to it.

Image generation and code changes remain the responsibility of the external agent/tooling. Markframe does not call AI models, edit images or code, send chat messages, perform OCR, or require a cloud service.

## Build

Requires Rust (edition 2024), Node.js 22.12+ and npm at build time:

```sh
npm ci --prefix frontend
npm run build --prefix frontend
cargo build --release --locked
```

The executable is `target/release/markframe` (`markframe.exe` on Windows). Copy that file to your PATH. It embeds all frontend assets; Node.js and the source directory are **not needed at runtime**. Rebuild the frontend before compiling Rust whenever UI files change.

## Run

```sh
markframe serve
# Open http://127.0.0.1:3741
markframe post output.png
markframe post output.png --server http://127.0.0.1:8080
```

`serve --port 8080` changes the port. `serve --host 0.0.0.0` explicitly exposes the server on the network; v0.1 has no authentication. The default is localhost only. Startup prints the listening URL. Set `RUST_LOG=markframe=debug` for diagnostics.

Runtime data is stored in the platform application data directory (Linux: `$XDG_DATA_HOME/markframe`, or `~/.local/share/markframe`; macOS: `~/Library/Application Support/dev.markframe.Markframe`; Windows: the platform local application-data directory). `MARKFRAME_DATA_DIR=/path/to/data` overrides it, including for isolated tests. Only one server may use a data directory at a time. Images use generated UUID filenames; `state.json` stores metadata and annotations and is replaced atomically after each mutation. Stop the server before copying the directory for backup. A corrupt state file fails startup rather than silently discarding data.

## Browser workflow

1. Start the server and open its URL.
2. Post PNG, JPEG, or WebP from a terminal or external agent. The browser switches automatically using SSE.
3. In **Rectangle** mode, drag on the image. Click a region to select, move, or resize it.
4. Enter a comment below the image. Changes save automatically. Failures preserve the input and offer **Retry**.
5. Press **Copy**, then paste into your existing AI conversation. It contains the exact comment followed by fenced JSON with `image_id`, `image_width`, `image_height`, and the native Annotorious annotation. Empty comments produce JSON only.
6. Use the header dropdown for previous images. New posts save pending edits before switching; a failed save keeps the current image open.

Use **+ / −**, **Fit**, or **Reset** to inspect the image. **Pan** enables drag-to-scroll without drawing. Scrollbars are also available. Coordinates always refer to intrinsic image pixels, regardless of zoom. Tab navigation reaches the comment and controls; Copy uses the browser Clipboard API (localhost or a secure context is required).

Annotations loaded into a browser are not live-synchronized with other editors; v0.1 is intended for one local reviewer. Refresh to load external API edits. SSE reconnects refresh the current image. Do not concurrently edit the same annotation in multiple browsers.

## HTTP API

```sh
curl -F 'image=@output.png' -F 'name=Optional label' http://127.0.0.1:3741/api/images
curl http://127.0.0.1:3741/api/current
curl http://127.0.0.1:3741/api/images/IMAGE_ID/annotations
```

| Method | Endpoint | Response |
| --- | --- | --- |
| GET | `/api/health` | `{"status":"ok"}` |
| POST | `/api/images` | Multipart `image`, optional `name`; 201 image metadata |
| GET | `/api/images` | `{"images":[...]}`, newest first |
| GET | `/api/current` | `{"image":{...}}`, or `{"image":null}` before any upload |
| GET | `/api/images/:id/content` | Original bytes and detected content type |
| GET | `/api/images/:id/annotations` | `{"image_id":"...","annotations":[...]}` |
| POST | `/api/images/:id/annotations` | Store envelope; 201 |
| GET | `/api/images/:id/annotations/:aid` | Envelope plus `image_id` |
| PUT | `/api/images/:id/annotations/:aid` | Replace existing envelope; 200 |
| DELETE | `/api/images/:id/annotations/:aid` | 204 |
| GET | `/api/images/:id/annotations/:aid/preview` | Full-size PNG with a visible rectangle, no comment text |
| GET | `/api/events` | SSE `event: image`, `data: {"id":"..."}` |

Image metadata contains `id`, sanitized `filename`, optional `name`, detected `mime_type`, intrinsic `width` and `height`, and RFC 3339 `created_at`. Image bytes are not rewritten. Uploads are limited to 25 MiB, 16,384 pixels per side and 32 megapixels, with a 256 MiB decoder allocation limit. Multipart overhead is limited to 64 KiB beyond the image limit. SVG, point annotations, and rotated rectangles are outside v0.1.

Annotation requests use native Annotorious v3 `ImageAnnotation` objects:

```json
{
  "schema": "annotorious-v3",
  "annotation": {
    "id": "example-annotation",
    "bodies": [{"id":"comment-1","annotation":"example-annotation","purpose":"commenting","value":"この円を少し大きく"}],
    "target": {
      "annotation": "example-annotation",
      "selector": {
        "type": "RECTANGLE",
        "geometry": {"x":10,"y":20,"w":80,"h":60,"bounds":{"minX":10,"minY":20,"maxX":90,"maxY":80}}
      }
    }
  }
}
```

Image IDs are UUIDs; annotation IDs use 1–128 ASCII letters, digits, `-`, or `_`. The annotation ID in a PUT must match its URL. Rectangles must fit within the image. Unknown native fields are preserved. Duplicate annotation creation returns 409; missing resources return 404; malformed requests return 4xx. Errors use `{"error":{"code":"...","message":"..."}}`. Image validation uses actual bytes rather than trusting uploaded Content-Type.

## Coding-agent skill

See [skills/markframe/SKILL.md](skills/markframe/SKILL.md). Copy its folder to your agent's skill directory if desired. It describes posting images and interpreting returned intrinsic-pixel annotations without depending on a particular AI product.

## Development and tests

```sh
# After the initial frontend build, run in separate terminals:
cargo run -- serve
npm run dev --prefix frontend
# Vite proxies /api to localhost:3741.

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
npm test --prefix frontend
npm run build --prefix frontend
```

Browser integration tests require Chromium: `cd frontend && npx playwright install chromium && npx playwright test`. Build the Rust debug binary first with `cargo build`. Tests start an isolated server and temporary data directory. They cover drawing, persistence, clipboard, SSE switching, history, zoom, and save failure recovery.

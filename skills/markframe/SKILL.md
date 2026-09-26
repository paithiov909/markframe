---
name: markframe
description: Post generated images to a running local Markframe server for visual review, manage posted images when requested, and interpret user-supplied Markframe annotations when revising the original work.
---
# Markframe

Markframe is a local visual review surface. It does not generate or edit images.

When an image needs the user's visual review and Markframe is available:

1. Save the image to disk using the appropriate external tool.
2. Post it with `markframe post path/to/image.png`. For a non-default server, append `--server http://127.0.0.1:PORT`. Equivalent HTTP: `curl -F 'image=@path/to/image.png' http://127.0.0.1:3741/api/images`.
3. Let the user inspect the image in Markframe. Do not reproduce it as ASCII or describe it unnecessarily.
4. Wait for the user's next instruction before modifying the image again.
5. Interpret supplied annotation geometry against the corresponding image's intrinsic `image_width` and `image_height`, not browser display dimensions. Preserve the user's comment verbatim.

`GET /api/current` identifies the latest image. `GET /api/images/IMAGE_ID/annotations` retrieves saved native Annotorious v3 envelopes. `GET /api/images/IMAGE_ID/annotations/ANNOTATION_ID/preview` provides the whole image with the selected rectangle marked, without rendering the comment into it.

If the server is not running, report the connection failure and explain that `markframe serve` starts it on `http://127.0.0.1:3741`. Posting retries are not idempotent: check `/api/current` if a response was lost before posting again.

Markframe never submits instructions to an AI service. Apply the user's feedback to the original source using the appropriate tooling, then post a new image for the next review.

## Manage review history

Use `markframe list --json` to inspect IDs, names, dates, annotation counts and image sizes. All management commands accept `--server URL`.

When the user requests deletion or cleanup:

- Inspect targets with `markframe delete IMAGE_ID... --dry-run --json`, `markframe prune --older-than 7d --keep-last 20 --dry-run --json`, or `markframe clear --dry-run --json`.
- Execute within the user's requested scope using `--yes --json`. Non-interactive deletion requires `--yes`; do not invoke an interactive prompt from an agent.
- `prune` preserves annotated images by default. Use `--include-annotated` only when deleting those images is within the requested scope. Age and keep-last filters combine with AND; at least one is required. Age units are `h`, `d`, `w`.
- `delete` and `clear` also remove annotations. The original posted file and settings remain untouched. There is no undo.
- Read `deleted_ids`, `missing_ids` and `failures`, not just the HTTP status. File cleanup can fail after metadata is deleted; subsequent deletion operations or server startup retry it. A partially failed CLI operation exits nonzero and retains its JSON report.

HTTP equivalents are `DELETE /api/images/ID?dry_run=true`, `POST /api/images/prune` with `older_than_seconds` and/or `keep_last`, and `DELETE /api/images` with `{"all":true}`. Batch bodies accept `dry_run`, and `candidate_ids` to restrict execution to a preview's `target_ids`; prune also accepts `include_annotated`. Recheck prune protection at execution. Do not edit the data directory directly. If separately previewing and executing via HTTP, pass the preview IDs to avoid deleting intervening posts.

Browser history follows deletions automatically. Unsaved feedback is retained in a copyable recovery panel until explicitly discarded, but does not survive closing/reloading the page.

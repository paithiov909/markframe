---
name: markframe
description: Post generated images to a running local Markframe server for visual review, and interpret user-supplied Markframe annotations when revising the original work.
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

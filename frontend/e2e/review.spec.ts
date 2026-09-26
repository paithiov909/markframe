import { test, expect } from "@playwright/test";
import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
let server: ChildProcess, dir: string;
const binary = resolve("../target/debug/markframe");
let fixture: Buffer;
test.beforeAll(async ({ browser }) => {
  dir = mkdtempSync(join(tmpdir(), "markframe-e2e-"));
  const page = await browser.newPage();
  const data = await page.evaluate(() => {
    const c = document.createElement("canvas");
    c.width = 800;
    c.height = 500;
    const ctx = c.getContext("2d")!;
    ctx.fillStyle = "#f4f1e8";
    ctx.fillRect(0, 0, 800, 500);
    ctx.fillStyle = "#446c9b";
    ctx.beginPath();
    ctx.arc(260, 240, 70, 0, Math.PI * 2);
    ctx.fill();
    ctx.beginPath();
    ctx.arc(480, 240, 70, 0, Math.PI * 2);
    ctx.fill();
    return c.toDataURL().split(",")[1];
  });
  fixture = Buffer.from(data, "base64");
  writeFileSync(join(dir, "sample.png"), fixture);
  await page.close();
  server = spawn(binary, ["serve", "--port", "37419"], {
    env: { ...process.env, MARKFRAME_DATA_DIR: join(dir, "data") },
    stdio: "pipe",
  });
  await expect
    .poll(async () => {
      try {
        return (await fetch("http://127.0.0.1:37419/api/health")).status;
      } catch {
        return 0;
      }
    })
    .toBe(200);
});
test.afterAll(async () => {
  if (server) {
    server.kill("SIGINT");
    await new Promise<void>((r) => server.once("exit", () => r()));
  }
  if (dir) rmSync(dir, { recursive: true, force: true });
});
test("post, draw, edit, copy, zoom, switch, recover and delete", async ({
  page,
  request,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  await expect(page.getByText("A place to look closer.")).toBeVisible();
  const first = JSON.parse(
    execFileSync(
      binary,
      ["post", join(dir, "sample.png"), "--server", "http://127.0.0.1:37419"],
      { encoding: "utf8" },
    ),
  );
  await expect(page.getByAltText("sample.png")).toBeVisible();
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  const box = (await page.getByAltText("sample.png").boundingBox())!;
  await page.mouse.move(box.x + 150, box.y + 120);
  await page.mouse.down();
  await page.mouse.move(box.x + 350, box.y + 300, { steps: 10 });
  await page.mouse.up();
  await expect(page.getByLabel("Selected annotation")).toBeEnabled();
  const comment = "  この円を少し大きく\nmake these circles farther apart  ";
  await page.getByLabel("Selected annotation").fill(comment);
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Copied",
  );
  const clipboard = await page.evaluate(() => navigator.clipboard.readText());
  expect(clipboard.startsWith(comment + "\n\n```json\n")).toBe(true);
  const parsed = JSON.parse(
    clipboard.slice(clipboard.indexOf("```json\n") + 8, -4),
  );
  expect(parsed.image_width).toBe(800);
  expect(parsed.image_height).toBe(500);
  let stored = (
    await (await request.get(`/api/images/${first.id}/annotations`)).json()
  ).annotations;
  expect(stored).toHaveLength(1);
  expect(stored[0].annotation.bodies[0].value).toBe(comment);
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  await page.getByRole("button", { name: "Fit", exact: true }).click();
  // Block saving to prove that a new image cannot discard an edit.
  await page.route("**/annotations/*", (route) =>
    route.request().method() === "PUT"
      ? route.fulfill({
          status: 500,
          contentType: "application/json",
          body: JSON.stringify({
            error: { message: "Simulated save failure" },
          }),
        })
      : route.continue(),
  );
  await page.getByLabel("Selected annotation").fill("unsaved 日本語");
  await expect(page.getByRole("alert")).toContainText("Simulated save failure");
  const second = await (
    await request.post("/api/images", {
      multipart: {
        image: { name: "second.png", mimeType: "image/png", buffer: fixture },
      },
    })
  ).json();
  await expect(page.getByRole("alert").first()).toContainText(
    "Simulated save failure",
  );
  await expect(page.getByAltText("sample.png")).toBeVisible();
  await expect(page.getByLabel("Selected annotation")).toHaveValue(
    "unsaved 日本語",
  );
  await page.unroute("**/annotations/*");
  await page
    .getByRole("button", { name: "Retry", exact: true })
    .first()
    .click();
  await expect(page.getByAltText("second.png")).toBeVisible();
  stored = (
    await (await request.get(`/api/images/${first.id}/annotations`)).json()
  ).annotations;
  expect(stored[0].annotation.bodies[0].value).toBe("unsaved 日本語");
  await page.getByLabel("Image history").selectOption(first.id);
  await expect(page.getByAltText("sample.png")).toBeVisible();
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  const region = page.locator(".a9s-annotation").first();
  await expect(region).toBeVisible();
  await region.click();
  await expect(page.getByLabel("Selected annotation")).toHaveValue(
    "unsaved 日本語",
  );
  await page.screenshot({ path: "test-results/review.png", fullPage: true });
  await page.getByRole("button", { name: "Delete", exact: true }).click();
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Deleted",
  );
  expect(
    (await (await request.get(`/api/images/${first.id}/annotations`)).json())
      .annotations,
  ).toHaveLength(0);
  expect((await request.get(`/api/images/${second.id}/content`)).status()).toBe(
    200,
  );
  expect(errors).toEqual([]);
});

test("intrinsic geometry survives zoom, move, resize and pan", async ({
  page,
  request,
}) => {
  const meta = await (
    await request.post("/api/images", {
      multipart: {
        image: { name: "zoom.png", mimeType: "image/png", buffer: fixture },
      },
    })
  ).json();
  await page.goto("/");
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  await page.getByRole("button", { name: "Zoom out", exact: true }).click();
  const box = (await page.getByAltText("zoom.png").boundingBox())!;
  const scale = box.width / 800;
  const drag = async (x: number, y: number, dx: number, dy: number) => {
    await page.mouse.move(box.x + x * scale, box.y + y * scale);
    await page.mouse.down();
    await page.mouse.move(box.x + (x + dx) * scale, box.y + (y + dy) * scale, {
      steps: 10,
    });
    await page.mouse.up();
  };
  await drag(100, 100, 200, 150);
  await page.getByLabel("Selected annotation").fill("zoom");
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  const geometry = async () => {
    const data = await (
      await request.get(`/api/images/${meta.id}/annotations`)
    ).json();
    return data.annotations[0]?.annotation.target.selector.geometry;
  };
  let g = await geometry();
  expect(g.x).toBeCloseTo(100, 0);
  expect(g.y).toBeCloseTo(100, 0);
  expect(g.w).toBeCloseTo(200, 0);
  expect(g.h).toBeCloseTo(150, 0);
  await drag(180, 170, 40, 30);
  await page.getByRole("button", { name: "Copy", exact: true }).click();
  await expect.poll(async () => (await geometry()).x).toBeCloseTo(140, 0);
  g = await geometry();
  expect(g.x).toBeCloseTo(140, 0);
  expect(g.y).toBeCloseTo(130, 0);
  const corner = page.locator(".a9s-corner-handle-bottomright");
  const handle = (await corner.boundingBox())!;
  await page.mouse.move(
    handle.x + handle.width / 2,
    handle.y + handle.height / 2,
  );
  await page.mouse.down();
  await page.mouse.move(
    handle.x + handle.width / 2 + 40 * scale,
    handle.y + handle.height / 2 + 30 * scale,
    { steps: 10 },
  );
  await page.mouse.up();
  await page.screenshot({ path: "test-results/resize.png" });
  await expect.poll(async () => (await geometry()).w).toBeCloseTo(240, 0);
  g = await geometry();
  expect(g.w).toBeCloseTo(240, 0);
  expect(g.h).toBeCloseTo(180, 0);
  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  await page.getByRole("button", { name: "Zoom in", exact: true }).click();
  await page.getByRole("button", { name: "Pan", exact: true }).click();
  const view = page.locator(".viewport");
  const v = (await view.boundingBox())!;
  await page.mouse.move(v.x + 400, v.y + 200);
  await page.mouse.down();
  await page.mouse.move(v.x + 200, v.y + 100, { steps: 10 });
  await page.mouse.up();
  expect(await view.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
  expect(
    (await (await request.get(`/api/images/${meta.id}/annotations`)).json())
      .annotations,
  ).toHaveLength(1);
});

test("a new post waits for an active rectangle gesture", async ({
  page,
  request,
}) => {
  const post = async (name: string) =>
    await (
      await request.post("/api/images", {
        multipart: { image: { name, mimeType: "image/png", buffer: fixture } },
      })
    ).json();
  const first = await post("drawing.png");
  await page.goto("/");
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  const box = (await page.getByAltText("drawing.png").boundingBox())!;
  await page.mouse.move(box.x + 100, box.y + 100);
  await page.mouse.down();
  await page.mouse.move(box.x + 250, box.y + 200, { steps: 10 });
  await post("incoming.png");
  await expect(page.getByAltText("drawing.png")).toBeVisible();
  await page.mouse.up();
  await expect(page.getByAltText("incoming.png")).toBeVisible();
  expect(
    (await (await request.get(`/api/images/${first.id}/annotations`)).json())
      .annotations,
  ).toHaveLength(1);
});

async function postReviewImage(
  request: import("@playwright/test").APIRequestContext,
  name: string,
) {
  const response = await request.post("/api/images", {
    multipart: { image: { name, mimeType: "image/png", buffer: fixture } },
  });
  expect(response.status()).toBe(201);
  return response.json();
}
function seededAnnotation() {
  return {
    schema: "annotorious-v3",
    annotation: {
      id: "recovery-region",
      bodies: [
        {
          id: "recovery-comment",
          annotation: "recovery-region",
          purpose: "commenting",
          value: "Saved comment",
        },
      ],
      target: {
        annotation: "recovery-region",
        selector: {
          type: "RECTANGLE",
          geometry: {
            x: 100,
            y: 100,
            w: 160,
            h: 100,
            bounds: { minX: 100, minY: 100, maxX: 260, maxY: 200 },
          },
        },
      },
    },
  };
}

test("external deletion keeps selection, falls back and clears the browser", async ({
  page,
  request,
}) => {
  await request.delete("/api/images", { data: { all: true } });
  const first = await postReviewImage(request, "first.png");
  const second = await postReviewImage(request, "second.png");
  const third = await postReviewImage(request, "third.png");
  await page.goto("/");
  await expect(page.getByAltText("third.png")).toBeVisible();
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  await page.getByLabel("Image history").selectOption(first.id);
  await expect(page.getByAltText("first.png")).toBeVisible();
  await request.delete(`/api/images/${second.id}`);
  await expect(page.getByLabel("Image history").locator("option")).toHaveCount(
    3,
  );
  await expect(page.getByAltText("first.png")).toBeVisible();
  await request.delete(`/api/images/${first.id}`);
  await expect(page.getByAltText("third.png")).toBeVisible();
  await expect(page.getByLabel("Image history")).toHaveValue(third.id);
  await request.delete("/api/images", { data: { all: true } });
  await expect(page.getByText("A place to look closer.")).toBeVisible();
  await expect(page.getByLabel("Image history").locator("option")).toHaveCount(
    1,
  );
});

test("deletion recovers unsaved feedback and ignores a late save response", async ({
  page,
  request,
}) => {
  await request.delete("/api/images", { data: { all: true } });
  await postReviewImage(request, "fallback.png");
  const current = await postReviewImage(request, "draft.png");
  const annotation = seededAnnotation();
  await request.post(`/api/images/${current.id}/annotations`, {
    data: annotation,
  });
  await page.goto("/");
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  const box = (await page.getByAltText("draft.png").boundingBox())!;
  await page.mouse.click(box.x + 160, box.y + 150);
  await expect(page.getByLabel("Selected annotation")).toHaveValue(
    "Saved comment",
  );
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let started = false;
  await page.route(
    `**/api/images/${current.id}/annotations/recovery-region`,
    async (route) => {
      if (route.request().method() !== "PUT") return route.continue();
      started = true;
      await gate;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: route.request().postData()!,
      });
    },
  );
  const comment = "  未保存のコメント\nKeep exact whitespace  ";
  await page.getByLabel("Selected annotation").fill(comment);
  await expect.poll(() => started).toBe(true);
  await request.delete(`/api/images/${current.id}`);
  await expect(page.getByAltText("fallback.png")).toBeVisible();
  const recovered = page.getByRole("region", { name: "Recovered drafts" });
  await expect(recovered).toBeVisible();
  await expect(recovered).toContainText(current.id);
  await recovered
    .getByRole("button", { name: "Copy recovered feedback" })
    .click();
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied.startsWith(comment + "\n\n```json\n")).toBe(true);
  expect(
    JSON.parse(copied.slice(copied.indexOf("```json\n") + 8, -4)).image_id,
  ).toBe(current.id);
  release();
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  await expect(recovered).toBeVisible();
  const prevented = await page.evaluate(() => {
    const event = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(prevented).toBe(true);
  await recovered
    .getByRole("button", { name: "Discard recovered feedback" })
    .click();
  await expect(recovered).toHaveCount(0);
  await expect(page.getByAltText("fallback.png")).toBeVisible();
});

test("reconnect reconciles missed deletion and annotation 404 does not delete an image", async ({
  page,
  request,
}) => {
  await request.delete("/api/images", { data: { all: true } });
  const current = await postReviewImage(request, "reconnect.png");
  await request.post(`/api/images/${current.id}/annotations`, {
    data: seededAnnotation(),
  });
  // Chromium's offline emulation can leave an existing SSE socket alive.
  // Keep real EventSource transport, but control disconnection/reconnection.
  await page.addInitScript(() => {
    const Native = window.EventSource;
    class Reconnectable extends EventTarget {
      onopen: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      source!: EventSource;
      constructor(readonly url: string) {
        super();
        (window as unknown as { reviewEvents: Reconnectable }).reviewEvents =
          this;
        this.connect();
      }
      connect() {
        this.source = new Native(this.url);
        this.source.onopen = (event) => this.onopen?.(event);
        this.source.onerror = (event) => this.onerror?.(event);
        for (const type of ["image", "images_deleted"]) {
          this.source.addEventListener(type, (event) =>
            this.dispatchEvent(new MessageEvent(type, { data: event.data })),
          );
        }
      }
      close() {
        this.source.close();
      }
      disconnect() {
        this.close();
        this.onerror?.(new Event("error"));
      }
    }
    window.EventSource = Reconnectable as unknown as typeof EventSource;
  });
  await page.goto("/");
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Saved",
  );
  const box = (await page.getByAltText("reconnect.png").boundingBox())!;
  await page.mouse.click(box.x + 160, box.y + 150);
  await page.route("**/annotations/recovery-region", (route) =>
    route.fulfill({
      status: 404,
      contentType: "application/json",
      body: JSON.stringify({ error: { message: "Annotation missing" } }),
    }),
  );
  await page.getByLabel("Selected annotation").fill("Retain this after 404");
  await expect(page.getByRole("status", { name: "Save status" })).toHaveText(
    "Unsaved changes",
  );
  await expect(page.getByAltText("reconnect.png")).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Recovered drafts" }),
  ).toHaveCount(0);
  await page.evaluate(() =>
    (
      window as unknown as { reviewEvents: { disconnect: () => void } }
    ).reviewEvents.disconnect(),
  );
  await expect(
    page.getByRole("alert").filter({ hasText: "Connection interrupted" }),
  ).toBeVisible();
  await request.delete(`/api/images/${current.id}`);
  await page.evaluate(() =>
    (
      window as unknown as { reviewEvents: { connect: () => void } }
    ).reviewEvents.connect(),
  );
  await expect(page.getByText("A place to look closer.")).toBeVisible({
    timeout: 15000,
  });
  const recovered = page.getByRole("region", { name: "Recovered drafts" });
  await expect(recovered).toContainText("Retain this after 404");
  await recovered
    .getByRole("button", { name: "Discard recovered feedback" })
    .click();
});

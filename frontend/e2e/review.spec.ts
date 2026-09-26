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

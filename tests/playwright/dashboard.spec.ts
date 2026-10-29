import { test, expect, Page } from "@playwright/test";
import { collectErrors, waitForHtmx } from "./helpers";

// rivet: verifies REQ-DASH-013
// The dashboard as a browser sees it. These exist because HTTP-level tests
// passed while, in a browser, Overview and Ask stayed empty (an out-of-band
// swap into a missing element, and an Ask button that fetched the whole
// page) and the footer claimed an SSE connection that never existed.

const VIEWS = [
  "Overview",
  "Ask",
  "Queue",
  "Critical Path",
  "Resources",
  "Outages",
  "Friction",
  "Schedule",
  "Health",
  "Agents",
  "Artifacts",
];

async function openView(page: Page, name: string) {
  await page.getByRole("button", { name, exact: true }).click();
  await waitForHtmx(page);
}

test.describe("assets", () => {
  test("htmx is served by warte as JavaScript", async ({ page }) => {
    const r = await page.request.get("/htmx.min.js");
    expect(r.status()).toBe(200);
    expect(r.headers()["content-type"]).toContain("javascript");
    expect(await r.text()).not.toMatch(/^</);
  });

  test("the page loads nothing from other origins", async ({ page, baseURL }) => {
    const foreign: string[] = [];
    page.on("request", (r) => {
      if (!r.url().startsWith(baseURL!)) foreign.push(r.url());
    });
    await page.goto("/");
    await page.waitForLoadState("networkidle");
    expect(foreign).toEqual([]);
  });
});

test.describe("views render", () => {
  test("overview shows content on load, with no errors", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await waitForHtmx(page);
    const content = page.locator("#content");
    await expect(content.getByRole("heading", { name: "Overview" })).toBeVisible();
    await expect(content).toContainText("Runners online");
    expect(errors).toEqual([]);
  });

  for (const name of VIEWS) {
    test(`${name} renders a non-empty view`, async ({ page }) => {
      const errors = collectErrors(page);
      await page.goto("/");
      await waitForHtmx(page);
      await openView(page, name);
      const text = (await page.locator("#content").innerText()).trim();
      expect(text.length, `${name} left #content empty`).toBeGreaterThan(20);
      expect(text).not.toContain("Loading...");
      // a view must never contain the whole page again
      await expect(page.locator("#content header")).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
});

test.describe("ask", () => {
  test("asks the ground station for a PR's ETA", async ({ page }) => {
    await page.goto("/");
    await openView(page, "Ask");
    await page.getByLabel("Repo:").fill("spar");
    await page.getByLabel("PR:").fill("511");
    await page.getByRole("button", { name: "Estimate" }).click();
    await waitForHtmx(page);
    const result = page.locator("#ask-result");
    await expect(result).toContainText("ETA for spar PR #511");
    await expect(result).toContainText("queued");
    // the form keeps what was asked
    await expect(page.getByLabel("Repo:")).toHaveValue("spar");
  });

  test("clicking an in-flight row asks for it, no typing", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await openView(page, "Ask");
    const row = page.locator("tr.pick", { hasText: "spar" }).first();
    await expect(row).toContainText("#511");
    await row.click();
    await waitForHtmx(page);
    await expect(page.locator("#ask-result")).toContainText("ETA for spar PR #511");
    await expect(page.getByLabel("Repo:")).toHaveValue("spar");
    await expect(page.getByLabel("PR:")).toHaveValue("511");
    expect(errors).toEqual([]);
  });
});

test.describe("honesty", () => {
  test("no claim of a live connection that does not exist", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("footer")).not.toContainText("SSE");
    await expect(page.locator("footer")).not.toContainText("connecting");
  });

  test("old fleet data is shown as stale", async ({ page }) => {
    await page.goto("/");
    await openView(page, "Queue");
    await expect(page.locator("#content")).toContainText("stale");
  });
});

test.describe("behind a path prefix (the ground-station host: https://<host>/warte/)", () => {
  test("every view works under /warte/", async ({ page, baseURL }) => {
    // Act as Caddy's `handle_path /warte/*`: strip the prefix and proxy.
    const outside: string[] = [];
    await page.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (!url.pathname.startsWith("/warte/")) {
        outside.push(url.pathname);
        return route.fulfill({ status: 404, body: "outside prefix" });
      }
      const upstream = baseURL + url.pathname.slice("/warte".length) + url.search;
      const response = await route.fetch({ url: upstream });
      await route.fulfill({ response });
    });
    const errors = collectErrors(page);
    await page.goto("/warte/");
    await waitForHtmx(page);
    await expect(page.locator("#content")).toContainText("Runners online");
    for (const name of ["Queue", "Ask", "Health"]) {
      await openView(page, name);
      expect((await page.locator("#content").innerText()).trim().length).toBeGreaterThan(20);
    }
    expect(outside, "requests escaped the /warte/ prefix").toEqual([]);
    expect(errors).toEqual([]);
  });
});

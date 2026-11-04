import { test, expect, Page } from "@playwright/test";
import { collectErrors, waitForHtmx } from "./helpers";

// rivet: verifies REQ-DASH-013
// The dashboard as a browser sees it. These exist because HTTP-level tests
// passed while, in a browser, Overview and Ask stayed empty (an out-of-band
// swap into a missing element, and an Ask button that fetched the whole
// page) and the footer claimed an SSE connection that never existed.

const VIEWS = [
  "Overview",
  "Jobs",
  "Compliance",
  "Ask",
  "Queue",
  "Machines",
  "Airport",
  "Critical Path",
  "Resources",
  "Outages",
  "Friction",
  "Schedule",
  "Health",
  "Costs",
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

test.describe("the view is in the URL", () => {
  test("a reload stays on the view, links open it, back goes back", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await waitForHtmx(page);
    await openView(page, "Machines");
    await expect(page).toHaveURL(/#machines$/);
    await page.reload();
    await waitForHtmx(page);
    await expect(page.getByRole("button", { name: "Machines", exact: true })).toHaveClass(/active/);
    await expect(page.locator("#content").getByRole("heading", { name: "Machines" })).toBeVisible();

    await page.goto("/#critical-path");
    await page.reload();
    await waitForHtmx(page);
    await expect(page.locator("#content").getByRole("heading", { name: "Critical Path" })).toBeVisible();

    await openView(page, "Queue");
    await page.goBack();
    await waitForHtmx(page);
    await expect(page.locator("#content").getByRole("heading", { name: "Critical Path" })).toBeVisible();
    expect(errors).toEqual([]);
  });
});

test.describe("critical path", () => {
  test("draws the declared chain and names its bottleneck", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await openView(page, "Critical Path");
    const chain = page.locator("#content .chain");
    await expect(chain).toContainText("jess");
    await expect(chain).toContainText("rivet");
    await expect(chain).toContainText("spar");
    await expect(page.locator("#content")).toContainText("Bottleneck: spar");
    expect(errors).toEqual([]);
  });
});

test.describe("airport", () => {
  test("the map draws the fleet; a sign tells its story and opens its view", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await waitForHtmx(page);
    await openView(page, "Airport");
    await expect(page.locator(".ap-pier").first()).toBeVisible();
    await expect(page.locator(".ap-towername")).toContainText("tower");
    // the runway field always has its lanes, busy or free
    expect(await page.locator(".ap-lane").count()).toBeGreaterThan(0);
    await page.locator(".ap-sign", { hasText: "Departures" }).click();
    await expect(page.locator(".ap-card .ap-title")).toHaveText("Departures");
    await page.locator(".ap-go").click();
    await waitForHtmx(page);
    await expect(page).toHaveURL(/#queue$/);
    expect(errors).toEqual([]);
  });

  test("a pier tells what it is; full screen fills the window", async ({ page }) => {
    const errors = collectErrors(page);
    await page.setViewportSize({ width: 1280, height: 720 });
    await page.goto("/#airport");
    await waitForHtmx(page);
    const label = page.locator(".ap-label").first();
    await expect(label).toBeVisible();
    const name = (await label.locator("b").textContent()) ?? "";
    await label.click();
    await expect(page.locator(".ap-card .ap-title")).toHaveText(name);
    await page.locator(".ap-fs").click();
    await expect.poll(() => page.evaluate(() => document.fullscreenElement?.id ?? "")).toBe("airport");
    // scaled to fit 1280x720 whole: full height, centred
    // the stage is redrawn on entering full screen; a detached one has no box
    await expect.poll(async () => Math.round((await page.locator(".ap-stage").boundingBox())?.height ?? 0)).toBe(720);
    const box = await page.locator(".ap-stage").boundingBox();
    expect(Math.round(box!.x)).toBe(Math.round((1280 - box!.width) / 2));
    await page.locator(".ap-fs").click();
    await expect.poll(() => page.evaluate(() => document.fullscreenElement === null)).toBe(true);
    expect(errors).toEqual([]);
  });
});

test.describe("airport field", () => {
  test("the field label stays above a full runway field", async ({ page }) => {
    const runways = Array.from({ length: 8 }, (_, i) => ({ id: `R0${i + 1}`, flight: `spar #${540 + i}`, job: "Build", elapsed_s: 300 + i * 200 }));
    await page.route("**/api/airport", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify({
      tower: "fourpi", runner_total: 14, holding: 3, runways,
      piers: [{ name: "fourpi", kind: "workshop", location: "lan", cores: 4, runners: 0, boards: [], agents: 0, ground_station: true, reporting: true, note: "" }],
    }) }));
    await page.goto("/#airport");
    await expect(page.locator(".ap-lane")).toHaveCount(7);
    const label = (await page.locator(".ap-fieldlabel").boundingBox())!;
    for (const sel of [".ap-tilt", ".ap-runsign"]) {
      for (const b of await page.locator(sel).all()) {
        const box = (await b.boundingBox())!;
        expect(label.y + label.height).toBeLessThanOrEqual(box.y + 1);
      }
    }
  });
});

test.describe("airport pilots", () => {
  test("agents walk and say what they do; a waiting one stands with a beacon", async ({ page }) => {
    const errors = collectErrors(page);
    const sessions = [
      { agent: "claude", repo: "warte", activity: "waiting for you", minutes: 30 },
      { agent: "claude", repo: "spar", activity: "running tests", minutes: 12 },
      { agent: "claude", repo: "relay", activity: "editing", minutes: 4 },
      { agent: "claude", repo: "rivet", activity: "", minutes: 90 },
    ];
    await page.route("**/api/airport", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify({
      tower: "fourpi", runner_total: 0, holding: 0, runways: [],
      piers: [{ name: "Mac", kind: "terminal", location: "lan", cores: 10, runners: 0, boards: [], agents: 4, sessions, ground_station: false, reporting: true, note: "" }],
    }) }));
    await page.goto("/#airport");
    // the three with a verb walk or wait; the one without a hook stays parked
    await expect(page.locator(".ap-walker")).toHaveCount(3);
    await expect(page.locator(".ap-walker.waiting .ap-say")).toHaveText("warte · waiting for you");
    await expect(page.locator(".ap-say", { hasText: "spar · running tests" })).toBeVisible();
    await expect(page.locator(".ap-beacon")).toHaveCount(1);
    // a walker stops while the pointer is on it, so it can be tapped
    const relay = page.locator(".ap-walker", { hasText: "relay" });
    await relay.hover({ force: true });
    await relay.click({ force: true });
    await expect(page.locator(".ap-card .ap-story")).toContainText("editing in relay");
    expect(errors).toEqual([]);
  });
});

test.describe("jobs", () => {
  test("overdue and missing jobs come first, mapped to their machine", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await openView(page, "Jobs");
    const rows = page.locator("#content tbody tr");
    await expect(rows).toHaveCount(3);
    await expect(rows.nth(0)).toContainText("backup");
    await expect(rows.nth(0)).toContainText("missing");
    await expect(rows.nth(1)).toContainText("archive");
    await expect(rows.nth(1)).toContainText("overdue");
    await expect(rows.nth(2)).toContainText("netcup");
    await expect(rows.nth(2)).toContainText("certbot");
    await expect(page.locator("#content .lede")).toContainText("2");
    const api = await (await page.request.get("/api/jobs")).json();
    expect(api.overdue).toBe(2);
    expect(errors).toEqual([]);
  });
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

  test("the answer refreshes on its own and leaves the form alone", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/");
    await openView(page, "Ask");
    await page.getByLabel("Repo:").fill("spar");
    await page.getByLabel("PR:").fill("511");
    await page.getByRole("button", { name: "Estimate" }).click();
    await expect(page.locator("#ask-result")).toContainText("ETA for spar PR #511");
    // half-typed, not submitted
    await page.getByLabel("Repo:").fill("riv");
    // the region's own refresh (the 30 s timer fires the same request); retried
    // until htmx has processed the swapped-in region
    let asked = false;
    page.on("request", (r) => {
      if (r.url().includes("/api/dashboard/ask-live?repo=spar&pr=511")) asked = true;
    });
    await expect(async () => {
      await page.evaluate(() => (window as any).htmx.trigger("#ask-live", "refresh"));
      expect(asked).toBe(true);
    }).toPass({ timeout: 5000 });
    await waitForHtmx(page);
    await expect(page.locator("#ask-result")).toContainText("ETA for spar PR #511");
    await expect(page.locator("#ask-live tr.pick").first()).toBeVisible();
    await expect(page.getByLabel("Repo:")).toHaveValue("riv");
    await expect(page.locator("#ask-live")).toHaveAttribute("hx-trigger", /every 30s/);
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

import { test, expect } from "@playwright/test";
import { collectErrors, waitForHtmx } from "./helpers";

// rivet: verifies REQ-DASH-018
// The idle scene: after two minutes without input a seasonal layer covers the
// dashboard, which stays visible; any input takes it away. The clock is
// Playwright's, set to St. Martin's evening in Berlin; the weather call is
// answered here, so the suite never leaves the machine.

test.use({ timezoneId: "Europe/Berlin" });

test.beforeEach(async ({ page }) => {
  await page.route("https://api.open-meteo.com/**", (r) =>
    r.fulfill({ json: { current: { temperature_2m: 6.4, weather_code: 61, wind_speed_10m: 12 } } }),
  );
  // 18:30 in Berlin on 11 November: after sunset, the day of St. Martin
  await page.clock.install({ time: new Date("2026-11-11T17:30:00Z") });
});

test("two idle minutes bring the season's scene; a move takes it away", async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto("/");
  await waitForHtmx(page);
  await page.clock.fastForward(60_000);
  await expect(page.locator("body")).not.toHaveClass(/idle-on/);

  await page.clock.fastForward(61_000);
  await expect(page.locator("body")).toHaveClass(/idle-on/);
  const cap = page.locator("#idle-caption");
  await expect(cap).toContainText("St. Martin");
  await expect(cap).toContainText("Lantern walks tonight");
  await expect(cap).toContainText("Sun up at");
  await expect(cap).toContainText("Moon:");
  await expect(cap).toContainText("Berlin: 6 °C, light rain");
  // the dashboard is still there underneath, and the layer takes no clicks
  await expect(page.locator("#content")).toBeVisible();
  await expect(page.locator("#idle-scene")).toHaveCSS("pointer-events", "none");

  await page.mouse.move(200, 200);
  await expect(page.locator("body")).not.toHaveClass(/idle-on/);
  expect(errors).toEqual([]);
});

test("?calm keeps the dashboard still", async ({ page }) => {
  await page.goto("/?calm");
  await waitForHtmx(page);
  await page.clock.fastForward(300_000);
  await expect(page.locator("body")).not.toHaveClass(/idle-on/);
  expect(await page.locator("#idle-scene").count()).toBe(0);
});

test("twelve themes, and the calendar knows Advent, the southern solstice and Easter", async ({ page }) => {
  await page.goto("/?calm");
  const t = await page.evaluate(() => {
    const w = (window as any).warteIdle;
    return w ? null : "loaded with calm";
  });
  // with ?calm the script returns before it defines anything
  expect(t).toBe("loaded with calm");
  await page.goto("/");
  const r = await page.evaluate(() => {
    const w = (window as any).warteIdle;
    return {
      months: w.months.map((m: string[]) => m[0]),
      advent: w.moment(new Date(2026, 10, 29), false).name,
      solsticeSouth: w.moment(new Date(2026, 11, 21), true).line,
      easter: w.easter(2027).toDateString(),
    };
  });
  expect(new Set(r.months).size).toBe(12);
  expect(r.advent).toBe("First Advent");
  expect(r.solsticeSouth).toContain("longest day");
  expect(r.easter).toBe("Sun Mar 28 2027");
});

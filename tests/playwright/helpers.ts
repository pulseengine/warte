import { Page } from "@playwright/test";

/** Wait for HTMX to finish all pending requests (as in rivet's helpers). */
export async function waitForHtmx(page: Page, timeout = 10_000) {
  await page.waitForFunction(() => !document.querySelector(".htmx-request"), {
    timeout,
  });
}

/** Collect uncaught page errors and console errors for the whole test. */
export function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (m) => {
    if (m.type() === "error" && !m.text().includes("favicon")) {
      errors.push(`console: ${m.text()}`);
    }
  });
  return errors;
}

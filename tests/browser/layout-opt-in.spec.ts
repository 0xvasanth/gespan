import { expect, test, type Page } from "@playwright/test";

test.use({ viewport: { width: 1300, height: 900 } });

async function openAppearance(page: Page) {
  const modifier = await page.evaluate(() =>
    /Mac|iPhone|iPad/.test(navigator.platform) ? "Meta" : "Control",
  );
  await page.keyboard.press(`${modifier}+,`);
  await page.getByRole("button", { name: "Appearance", exact: true }).click();
}

test("opts in from the invitation and switches back without changing glass", async ({
  page,
}, testInfo) => {
  await page.addInitScript(() => {
    localStorage.setItem("monocode.insetGlass", "0");
  });
  await page.goto("/tests/browser/workspace-performance.html");
  const notice = page.getByRole("status", { name: "Try the new layout" });
  await expect(notice).toBeVisible();
  await expect(page.locator(".inset-shell")).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath("classic-invitation.png"),
  });

  await notice.getByRole("button", { name: "Try it", exact: true }).click();
  await expect(page.locator(".inset-shell")).toBeVisible();
  await expect(notice).toHaveCount(0);
  await expect(
    page.locator('[data-session-drop="workspace-session-0"]'),
  ).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("new-layout.png") });

  await page.reload();
  await expect(page.locator(".inset-shell")).toBeVisible();
  await expect(notice).toHaveCount(0);
  await openAppearance(page);
  await expect(
    page.getByRole("switch", { name: "Inset workspace glass", exact: true }),
  ).not.toBeChecked();
  const layout = page.getByRole("switch", { name: "New layout", exact: true });
  await expect(layout).toBeChecked();
  await layout.click();
  await expect(layout).not.toBeChecked();
  await expect(page.locator(".inset-shell")).toHaveCount(0);
  expect(
    await page.evaluate(() => localStorage.getItem("monocode.insetGlass")),
  ).toBe("0");

  await page.reload();
  await expect(
    page.locator('[data-session-drop="workspace-session-0"]'),
  ).toBeVisible();
  await expect(page.locator(".inset-shell")).toHaveCount(0);
  await expect(notice).toHaveCount(0);
});

test("remembers Not now and still allows opting in from Settings", async ({
  page,
}) => {
  await page.goto("/tests/browser/workspace-performance.html");
  const notice = page.getByRole("status", { name: "Try the new layout" });
  await notice.getByRole("button", { name: "Not now", exact: true }).click();
  await expect(notice).toHaveCount(0);
  await expect(page.locator(".inset-shell")).toHaveCount(0);
  await page.reload();
  await expect(
    page.locator('[data-session-drop="workspace-session-0"]'),
  ).toBeVisible();
  await expect(notice).toHaveCount(0);
  await openAppearance(page);
  const layout = page.getByRole("switch", { name: "New layout", exact: true });
  await expect(layout).not.toBeChecked();
  await layout.click();
  await expect(layout).toBeChecked();
  await expect(page.locator(".inset-shell")).toBeVisible();
  await expect(notice).toHaveCount(0);
});

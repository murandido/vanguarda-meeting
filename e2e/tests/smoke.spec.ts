import { expect, test } from "@playwright/test";
import { abrirQuadro, quadroUnico } from "./canvas";

test("home carrega e mostra Cartazes e Ritmo", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Cartazes" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Ritmo" })).toBeVisible();
});

test("página do quadro conecta (status online)", async ({ page }) => {
  const quadro = quadroUnico("smoke");
  await abrirQuadro(page, quadro);
  await expect(page.locator("#board")).toHaveAttribute("data-board", quadro);
});

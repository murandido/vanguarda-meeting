import { expect, test } from "@playwright/test";
import { abrirQuadro, entrar, quadroUnico } from "./canvas";

test("sem login, vai para a página de entrar", async ({ page }) => {
  await page.goto("/b/qualquer");
  await expect(page).toHaveURL("/login");
  await expect(page.getByRole("link", { name: "Entrar com GitHub" })).toBeVisible();
});

test("home carrega e mostra Cartazes, Ritmo e o usuário", async ({ page }) => {
  await entrar(page, "ana");
  await expect(page.getByRole("heading", { name: "Cartazes" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Ritmo" })).toBeVisible();
  await expect(page.locator(".user span")).toHaveText("ana");
});

test("sair encerra a sessão", async ({ page }) => {
  await entrar(page, "ana");
  await page.getByRole("button", { name: "Sair" }).click();
  await expect(page).toHaveURL("/login");
  await page.goto("/");
  await expect(page).toHaveURL("/login");
});

test("página do quadro conecta (status online)", async ({ page }) => {
  const quadro = quadroUnico("smoke");
  await abrirQuadro(page, quadro);
  await expect(page.locator("#board")).toHaveAttribute("data-board", quadro);
});

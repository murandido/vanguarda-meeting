import { expect, test } from "@playwright/test";
import { abrirQuadro, criarQuadro, entrar, quadroUnico } from "./canvas";

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
  await entrar(page, "ana");
  expect(await criarQuadro(page, quadro)).toBe(quadro);
  await abrirQuadro(page, quadro);
  await expect(page.locator("#board")).toHaveAttribute("data-board", quadro);
});

test("quadro inexistente dá 404", async ({ page }) => {
  await entrar(page, "ana");
  const res = await page.goto(`/b/${quadroUnico("nao-existe")}`);
  expect(res?.status()).toBe(404);
  await expect(page.getByRole("heading", { name: "Quadro não encontrado" })).toBeVisible();
});

test("Cartazes: criar, renomear, arquivar e desarquivar pela home", async ({ page }) => {
  await entrar(page, "ana");
  const titulo = `Reunião ${quadroUnico("ui")}`;
  const cartazes = page.locator("#cartazes");

  await cartazes.getByLabel("Título do novo quadro").fill(titulo);
  await cartazes.getByRole("button", { name: "Novo quadro" }).click();
  const link = cartazes.getByRole("link", { name: titulo });
  await expect(link).toBeVisible();
  const href = await link.getAttribute("href");
  expect(href).toMatch(/^\/b\/reuniao-ui-/);

  const item = cartazes.locator("li", { has: page.getByRole("link", { name: titulo }) });
  await item.getByText("Renomear").click();
  await item.getByRole("textbox").fill(`${titulo} (editado)`);
  await item.getByRole("button", { name: "Salvar" }).click();
  const renomeado = cartazes.getByRole("link", { name: `${titulo} (editado)` });
  await expect(renomeado).toBeVisible();

  await cartazes
    .locator("li", { has: page.getByRole("link", { name: `${titulo} (editado)` }) })
    .getByRole("button", { name: "Arquivar" })
    .click();
  await expect(renomeado).toHaveCount(0);
  expect((await page.request.get(href!)).status()).toBe(410);

  await cartazes.getByText(/Arquivados/).click();
  await cartazes
    .locator("li", { hasText: `${titulo} (editado)` })
    .getByRole("button", { name: "Desarquivar" })
    .click();
  await cartazes.getByRole("link", { name: `${titulo} (editado)` }).click();
  await expect(page).toHaveURL(href!);
  await expect(page.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
});

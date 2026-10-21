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

// Walks every page type under the real CSP (login, home with the HTMX board
// list, a board with WASM + WebSocket, Ritmo with SSE, check-in, logout) and
// fails on any violation, reported either as an event or on the console.
test("CSP: nenhuma violação em nenhuma página", async ({ page }) => {
  const violations: string[] = [];
  page.on("console", (msg) => {
    if (/Content[- ]Security[- ]Policy/i.test(msg.text())) violations.push(`console: ${msg.text()}`);
  });
  await page.exposeFunction("reportCspViolation", (v: string) => violations.push(v));
  await page.addInitScript(() => {
    document.addEventListener("securitypolicyviolation", (e) => {
      const w = window as unknown as { reportCspViolation: (v: string) => void };
      w.reportCspViolation(`${e.violatedDirective} ${e.blockedURI} ${e.sourceFile}:${e.lineNumber}`);
    });
  });

  const login = await page.goto("/login");
  expect(login?.headers()["content-security-policy"]).toContain("frame-ancestors 'none'");
  await entrar(page, "ana");

  // Home: the board list arrives through HTMX; create a board through it.
  const titulo = quadroUnico("csp");
  const cartazes = page.locator("#cartazes");
  await cartazes.getByLabel("Título do novo quadro").fill(titulo);
  await cartazes.getByRole("button", { name: "Novo quadro" }).click();
  await cartazes.getByRole("link", { name: titulo }).click();
  await expect(page.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
  await page.mouse.dblclick(400, 300);

  // Ritmo: project colors come from data attributes, and the new-task form
  // is reset by ritmo.js (no inline handlers).
  await page.goto("/ritmo");
  const config = page.locator("details.config");
  if (!(await config.evaluate((el) => (el as HTMLDetailsElement).open))) {
    await config.locator("summary").click();
  }
  const projeto = `Projeto ${titulo}`;
  await page.getByLabel("Nome do projeto").fill(projeto);
  await page.getByLabel("Cor do projeto").fill("#ff8800");
  await page.getByRole("button", { name: "Criar projeto" }).click();
  await expect(page.getByLabel("Projeto", { exact: true })).toContainText(projeto);
  await page.getByLabel("Título da tarefa").fill(`Tarefa ${titulo}`);
  await page.getByLabel("Projeto", { exact: true }).selectOption({ label: projeto });
  await page.getByRole("button", { name: "Adicionar" }).click();
  const card = page.locator(".card", { hasText: `Tarefa ${titulo}` });
  await expect(card).toBeVisible();
  await expect(page.getByLabel("Título da tarefa")).toHaveValue("");
  await expect(page.getByLabel("Título da tarefa")).toBeFocused();
  expect(await card.evaluate((el) => getComputedStyle(el).getPropertyValue("--cor").trim())).toBe(
    "#ff8800",
  );

  await page.getByRole("link", { name: "Check-in de hoje" }).click();
  await expect(page).toHaveURL("/ritmo/checkin");
  const item = page.locator("fieldset", { hasText: `Tarefa ${titulo}` });
  expect(await item.evaluate((el) => getComputedStyle(el).getPropertyValue("--cor").trim())).toBe(
    "#ff8800",
  );

  await page.getByRole("button", { name: "Sair" }).click();
  await expect(page).toHaveURL("/login");
  expect(violations).toEqual([]);
});

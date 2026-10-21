// Ritmo: a Ana cria uma tarefa e faz check-in; a Bia, com a página aberta,
// vê as duas coisas chegarem ao vivo (SSE), sem recarregar.
import { expect, test } from "@playwright/test";
import { entrar } from "./canvas";

test("ana cria tarefa e faz check-in; bia vê ao vivo", async ({ browser }) => {
  const sufixo = `${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
  const projeto = `Projeto ${sufixo}`;
  const tarefa = `Tarefa ${sufixo}`;
  const nota = `Nota ${sufixo}`;

  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  try {
    const a = await ctxA.newPage();
    const b = await ctxB.newPage();
    await entrar(a, "ana");
    await entrar(b, "bia");

    // Bia abre o Ritmo pela home e fica olhando.
    await b.getByRole("link", { name: "Abrir o Ritmo" }).click();
    await expect(b).toHaveURL("/ritmo");
    await expect(b.locator("#quadro")).toBeVisible();

    // Ana cria um projeto (a página recarrega) e uma tarefa.
    await a.goto("/ritmo");
    const config = a.locator("details.config");
    if (!(await config.evaluate((el) => (el as HTMLDetailsElement).open))) {
      await config.locator("summary").click();
    }
    await a.getByLabel("Nome do projeto").fill(projeto);
    await a.getByRole("button", { name: "Criar projeto" }).click();
    await expect(a.getByLabel("Projeto", { exact: true })).toContainText(projeto);

    await a.getByLabel("Título da tarefa").fill(tarefa);
    await a.getByLabel("Projeto", { exact: true }).selectOption({ label: projeto });
    await a.getByRole("button", { name: "Adicionar" }).click();
    const raiaAnaEmA = a.locator('.raia[data-login="ana"] .coluna[data-status="todo"]');
    await expect(raiaAnaEmA).toContainText(tarefa);

    // Bia vê a tarefa da Ana sem recarregar.
    const raiaAnaEmB = b.locator('.raia[data-login="ana"]');
    await expect(raiaAnaEmB).toContainText(tarefa, { timeout: 10_000 });

    // Ana move para Fazendo e faz o check-in.
    await raiaAnaEmA
      .locator(".card", { hasText: tarefa })
      .getByRole("button", { name: "Mover para Fazendo" })
      .click();
    await expect(
      a.locator('.raia[data-login="ana"] .coluna[data-status="doing"]'),
    ).toContainText(tarefa);

    await a.getByRole("link", { name: "Check-in de hoje" }).click();
    await expect(a).toHaveURL("/ritmo/checkin");
    const item = a.locator("fieldset", { hasText: tarefa });
    await item.getByLabel("avancei").check();
    await item.getByRole("textbox").fill(nota);
    await a.getByRole("button", { name: "Salvar check-in" }).click();
    await expect(a).toHaveURL("/ritmo");
    await expect(a.getByRole("link", { name: "Editar check-in de hoje" })).toBeVisible();

    // Bia vê o check-in da Ana no feed, também ao vivo.
    const feedAna = b.locator('.feed [data-login="ana"]');
    await expect(feedAna).toContainText(nota, { timeout: 10_000 });
    await expect(feedAna).toContainText("avancei");
    await expect(
      b.locator('.raia[data-login="ana"] .coluna[data-status="doing"]'),
    ).toContainText(tarefa);
  } finally {
    await ctxA.close();
    await ctxB.close();
  }
});

test("ritmo cabe no celular (375 px)", async ({ browser }) => {
  const ctx = await browser.newContext({ viewport: { width: 375, height: 740 } });
  try {
    const page = await ctx.newPage();
    await entrar(page, "bia");
    await page.goto("/ritmo");
    await expect(page.getByRole("link", { name: /check-in de hoje/i })).toBeVisible();
    const largura = await page.evaluate(() => document.documentElement.scrollWidth);
    expect(largura).toBeLessThanOrEqual(375);
  } finally {
    await ctx.close();
  }
});

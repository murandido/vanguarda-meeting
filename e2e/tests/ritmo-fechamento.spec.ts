// Ritmo: streaks (KB-06), week closing (KB-07) and celebrations (KB-08).
//
// The server runs with DEV_TODAY=2026-10-20, a Tuesday (the meeting day), so
// the current week (14/10 – 20/10) can be closed. "caio" and "dani" are only
// used here: closing makes a week read-only for its owner.
import { expect, type Page, test } from "@playwright/test";
import { entrar } from "./canvas";

const sufixo = () => `${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;

/** Creates a project from the settings panel; the page reloads. */
async function criarProjeto(page: Page, nome: string): Promise<void> {
  const config = page.locator("details.config");
  if (!(await config.evaluate((el) => (el as HTMLDetailsElement).open))) {
    await config.locator("summary").click();
  }
  await page.getByLabel("Nome do projeto").fill(nome);
  await page.getByRole("button", { name: "Criar projeto" }).click();
  await expect(page.getByLabel("Projeto", { exact: true })).toContainText(nome);
}

async function criarTarefa(page: Page, projeto: string, titulo: string): Promise<void> {
  await page.getByLabel("Título da tarefa").fill(titulo);
  await page.getByLabel("Projeto", { exact: true }).selectOption({ label: projeto });
  await page.getByRole("button", { name: "Adicionar" }).click();
  await expect(page.locator(".card", { hasText: titulo })).toBeVisible();
}

async function mover(page: Page, titulo: string, para: string): Promise<void> {
  await page
    .locator(".card", { hasText: titulo })
    .getByRole("button", { name: `Mover para ${para}` })
    .click();
}

test("check-in aumenta o streak e comemora", async ({ browser }) => {
  const ctx = await browser.newContext();
  try {
    const page = await ctx.newPage();
    await entrar(page, "bia");
    await page.goto("/ritmo");
    const streak = page.locator('.streak[data-login="bia"]');
    await expect(streak).toBeVisible();
    const antes = Number(await streak.locator(".dias").textContent());
    const jaFeito = (await streak.getAttribute("data-status")) === "feito";

    await page.getByRole("link", { name: /check-in de hoje/i }).click();
    await expect(page).toHaveURL("/ritmo/checkin");
    await page.getByRole("button", { name: /(Salvar|Atualizar) check-in/ }).click();

    await expect(page).toHaveURL("/ritmo");
    await expect(page.locator("#comemoracao")).toContainText("Check-in salvo!");
    await expect(streak).toHaveAttribute("data-status", "feito");
    await expect(streak.locator(".dias")).toHaveText(String(jaFeito ? antes : antes + 1));

    // The home card shows it too.
    await page.goto("/");
    await expect(page.locator('.streaks-home [data-login="você"]')).toContainText(
      String(jaFeito ? antes : antes + 1),
    );
  } finally {
    await ctx.close();
  }
});

test("fechar a semana leva a tarefa para a próxima (celular)", async ({ browser }) => {
  const ctx = await browser.newContext({ viewport: { width: 375, height: 740 } });
  try {
    const page = await ctx.newPage();
    const s = sufixo();
    const projeto = `Projeto ${s}`;
    const feita = `Feita ${s}`;
    const levada = `Levada ${s}`;
    await entrar(page, "caio");
    await page.goto("/ritmo");
    await criarProjeto(page, projeto);
    await criarTarefa(page, projeto, feita);
    await criarTarefa(page, projeto, levada);

    // Done: confetti (motion is allowed in this context).
    await mover(page, feita, "Fazendo");
    await mover(page, feita, "Feito");
    await expect(page.locator("#confete")).toBeAttached();
    await expect(page.locator("#comemoracao")).toContainText("Feito!");

    const raia = page.locator('.raia[data-login="caio"]');
    await raia.getByRole("link", { name: "Fechar semana" }).click();
    await expect(page).toHaveURL("/ritmo/semana/fechar?semana=2026-10-14");
    const resumo = page.locator('.resumo[data-login="caio"]');
    await expect(resumo.locator("dl")).toContainText("Extras feitos1");
    await expect(resumo.locator("fieldset")).toHaveCount(1);
    await resumo.locator("fieldset", { hasText: levada }).getByLabel("levar para a próxima").check();
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
    await resumo.getByRole("button", { name: "Fechar semana" }).click();

    // Next week: the task is there, marked as carried over.
    await expect(page).toHaveURL("/ritmo?semana=2026-10-21");
    const card = page.locator('.raia[data-login="caio"] .card', { hasText: levada });
    await expect(card).toBeVisible();
    await expect(card).toContainText("levada");

    // The closed week is read-only for caio.
    await page.goto("/ritmo");
    await expect(raia).toContainText("Semana fechada");
    await expect(raia.getByRole("button", { name: /Mover para/ })).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
  } finally {
    await ctx.close();
  }
});

test("sem confete com prefers-reduced-motion", async ({ browser }) => {
  const ctx = await browser.newContext({ reducedMotion: "reduce" });
  try {
    const page = await ctx.newPage();
    const s = sufixo();
    const projeto = `Projeto ${s}`;
    const tarefa = `Tarefa ${s}`;
    await entrar(page, "dani");
    await page.goto("/ritmo");
    await criarProjeto(page, projeto);
    await criarTarefa(page, projeto, tarefa);
    // Watch for the canvas the whole time, in case it is removed quickly.
    await page.evaluate(() => {
      const w = window as unknown as { confeteVisto: boolean };
      w.confeteVisto = false;
      new MutationObserver(() => {
        if (document.getElementById("confete")) w.confeteVisto = true;
      }).observe(document.body, { childList: true });
    });

    await mover(page, tarefa, "Fazendo");
    await mover(page, tarefa, "Feito");
    // The message still shows (and is announced), without the animation.
    await expect(page.locator("#comemoracao")).toContainText("Feito!");
    await expect(page.locator("#confete")).toHaveCount(0);
    expect(
      await page.evaluate(() => (window as unknown as { confeteVisto: boolean }).confeteVisto),
    ).toBe(false);
  } finally {
    await ctx.close();
  }
});

// Utilitários para inspecionar o canvas do quadro via pixels.
// O board.js só desenha no canvas, então o estado é verificado lendo a cor
// de pontos específicos (com deviceScaleFactor 1, coordenada CSS == pixel).
import { expect, type Browser, type Page } from "@playwright/test";

/** Cor de preenchimento dos post-its (#ffe066). */
export const COR_POSTIT = [255, 224, 102] as const;

export type RGBA = [number, number, number, number];

/** Lê o pixel RGBA do canvas #board na posição (x, y). */
export async function pixel(page: Page, x: number, y: number): Promise<RGBA> {
  return page.evaluate(
    ([px, py]) => {
      const c = document.getElementById("board") as HTMLCanvasElement;
      const d = c.getContext("2d")!.getImageData(px, py, 1, 1).data;
      return [d[0], d[1], d[2], d[3]] as [number, number, number, number];
    },
    [x, y] as const,
  );
}

/** Verdadeiro se o pixel é (aproximadamente) a cor do post-it e opaco. */
export function ehPostit([r, g, b, a]: RGBA): boolean {
  const tol = 3;
  return (
    a > 250 &&
    Math.abs(r - COR_POSTIT[0]) <= tol &&
    Math.abs(g - COR_POSTIT[1]) <= tol &&
    Math.abs(b - COR_POSTIT[2]) <= tol
  );
}

/** Espera (com poll) até o pixel em (x, y) ser ou não ser post-it. */
export const esperaPostit = (page: Page, x: number, y: number) =>
  expect.poll(async () => ehPostit(await pixel(page, x, y)), { timeout: 5_000 });

/**
 * Conta pixels coloridos (saturados) numa caixa `w`x`h` a partir de (x, y).
 * A grade de pontos do fundo é cinza (r == g == b) e não conta; os cursores
 * dos outros usuários são coloridos.
 */
export async function pixelsColoridos(page: Page, x: number, y: number, w = 14, h = 14): Promise<number> {
  return page.evaluate(
    ([cx, cy, cw, ch]) => {
      const c = document.getElementById("board") as HTMLCanvasElement;
      const d = c.getContext("2d")!.getImageData(cx, cy, cw, ch).data;
      let n = 0;
      for (let i = 0; i < d.length; i += 4) {
        const croma = Math.max(d[i], d[i + 1], d[i + 2]) - Math.min(d[i], d[i + 1], d[i + 2]);
        if (d[i + 3] > 0 && croma > 40) n++;
      }
      return n;
    },
    [x, y, w, h] as const,
  );
}

/** Entra pelo login de desenvolvimento (DEV_LOGIN=1 no webServer). */
export async function entrar(page: Page, login: string): Promise<void> {
  await page.goto(`/auth/dev/${login}`);
  await expect(page).toHaveURL("/");
}

/**
 * Cria o quadro pelo mesmo POST que o HTMX faz na home. O título já é um
 * slug válido, então o id do quadro é igual a ele. Criar um quadro que já
 * existe gera outro id (`-2`), por isso só um dos usuários cria.
 */
export async function criarQuadro(page: Page, titulo: string): Promise<string> {
  const res = await page.request.post("/cartazes", {
    form: { titulo },
    headers: { "HX-Request": "true" },
  });
  expect(res.status()).toBe(200);
  return res.headers()["x-board-id"];
}

/** Entra como `login`, abre o quadro e espera o WebSocket ficar online. */
export async function abrirQuadro(page: Page, quadro: string, login = "ana"): Promise<void> {
  await entrar(page, login);
  await page.goto(`/b/${quadro}`);
  await expect(page.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
}

/** Entra como `login`, cria um quadro novo e o abre (quadros não nascem mais pela URL). */
export async function abrirNovoQuadro(page: Page, quadro: string, login = "ana"): Promise<void> {
  await entrar(page, login);
  expect(await criarQuadro(page, quadro)).toBe(quadro);
  await page.goto(`/b/${quadro}`);
  await expect(page.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
}

/** Ana e Bia no mesmo quadro, em contextos isolados (cookies separados). */
export async function doisUsuarios(browser: Browser, quadro: string) {
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  await abrirNovoQuadro(a, quadro, "ana");
  await abrirQuadro(b, quadro, "bia");
  return {
    a,
    b,
    fechar: async () => {
      await ctxA.close();
      await ctxB.close();
    },
  };
}

/** Texto do indicador de zoom da toolbar (ex.: "100%"). */
export const zoom = (page: Page) => page.locator("#zoom");

/** Nome de quadro único por teste, para isolar o estado entre execuções. */
export function quadroUnico(prefixo = "e2e"): string {
  return `${prefixo}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

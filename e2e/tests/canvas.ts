// Utilitários para inspecionar o canvas do quadro via pixels.
// O board.js só desenha no canvas, então o estado é verificado lendo a cor
// de pontos específicos (com deviceScaleFactor 1, coordenada CSS == pixel).
import { expect, type Page } from "@playwright/test";

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

/** Entra pelo login de desenvolvimento (DEV_LOGIN=1 no webServer). */
export async function entrar(page: Page, login: string): Promise<void> {
  await page.goto(`/auth/dev/${login}`);
  await expect(page).toHaveURL("/");
}

/** Entra como `login`, abre o quadro e espera o WebSocket ficar online. */
export async function abrirQuadro(page: Page, quadro: string, login = "ana"): Promise<void> {
  await entrar(page, login);
  await page.goto(`/b/${quadro}`);
  await expect(page.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
}

/** Nome de quadro único por teste, para isolar o estado entre execuções. */
export function quadroUnico(prefixo = "e2e"): string {
  return `${prefixo}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

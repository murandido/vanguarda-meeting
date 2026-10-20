// Gate da Fase 0: dois usuários (contextos separados) no mesmo quadro
// enxergam criação, movimento e persistência dos post-its via Loro + WS.
import { expect, test, type Browser, type Page } from "@playwright/test";
import { abrirQuadro, criarQuadro, ehPostit, entrar, pixel, quadroUnico } from "./canvas";

/** Cria dois "usuários" em contextos isolados (localStorage separado). */
async function doisUsuarios(browser: Browser, quadro: string) {
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  await entrar(a, "ana");
  expect(await criarQuadro(a, quadro)).toBe(quadro);
  await Promise.all([abrirQuadro(a, quadro, "ana"), abrirQuadro(b, quadro, "bia")]);
  return {
    a,
    b,
    fechar: async () => {
      await ctxA.close();
      await ctxB.close();
    },
  };
}

const esperaPostit = (page: Page, x: number, y: number) =>
  expect.poll(async () => ehPostit(await pixel(page, x, y)), { timeout: 5_000 });

test("criar, mover e persistir post-it entre dois usuários", async ({ browser }) => {
  const quadro = quadroUnico();
  const { a, b, fechar } = await doisUsuarios(browser, quadro);
  try {
    // O cursor de A é desenhado no canvas de B como um triângulo que parte da
    // ponta do ponteiro para baixo/direita (x..x+12, y..y+12). Por isso as
    // sondas ficam dentro do retângulo, mas acima/à direita do ponto clicado
    // e fora do caminho do arraste. Tudo bem abaixo da toolbar fixa (~40px).
    // Retângulo criado em (400,300) ocupa x 340..460, y 260..340.
    const SONDA_ORIGEM = [440, 275] as const;
    // Depois de arrastar para (700,500) ocupa x 640..760, y 460..540.
    const SONDA_DESTINO = [740, 475] as const;

    await esperaPostit(b, ...SONDA_ORIGEM).toBe(false);

    // A cria um retângulo 120x80 centrado no duplo clique.
    await a.mouse.dblclick(400, 300);
    await esperaPostit(a, ...SONDA_ORIGEM).toBe(true);
    await esperaPostit(b, ...SONDA_ORIGEM).toBe(true);

    // A arrasta o retângulo de (400,300) para (700,500).
    await a.mouse.move(400, 300);
    await a.mouse.down();
    await a.mouse.move(700, 500, { steps: 10 });
    await a.mouse.up();

    await esperaPostit(b, ...SONDA_DESTINO).toBe(true);
    await esperaPostit(b, ...SONDA_ORIGEM).toBe(false);

    // Persistência: B recarrega e recebe o snapshot do servidor.
    await b.reload();
    await expect(b.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
    await esperaPostit(b, ...SONDA_DESTINO).toBe(true);
    await esperaPostit(b, ...SONDA_ORIGEM).toBe(false);
  } finally {
    await fechar();
  }
});

test("presença: cursor de A aparece no canvas de B", async ({ browser }) => {
  const quadro = quadroUnico("presenca");
  const { a, b, fechar } = await doisUsuarios(browser, quadro);
  try {
    const [x, y] = [600, 400];

    // Conta pixels não transparentes numa caixa logo abaixo/à direita do
    // ponteiro, onde o triângulo do cursor é desenhado (x..x+12, y..y+12).
    const pixelsDesenhados = () =>
      b.evaluate(
        ([cx, cy]) => {
          const c = document.getElementById("board") as HTMLCanvasElement;
          const d = c.getContext("2d")!.getImageData(cx, cy, 14, 14).data;
          let n = 0;
          for (let i = 3; i < d.length; i += 4) if (d[i] > 0) n++;
          return n;
        },
        [x, y] as const,
      );

    expect(await pixelsDesenhados()).toBe(0);

    // O board.js limita o envio de cursor a ~30/s; dentro do poll movemos o
    // mouse de A de novo (alternando 1px) para garantir que um envio saia.
    let alterna = 0;
    await expect
      .poll(
        async () => {
          alterna ^= 1;
          await a.mouse.move(x - alterna, y);
          await a.waitForTimeout(50);
          await a.mouse.move(x, y);
          return pixelsDesenhados();
        },
        { timeout: 5_000 },
      )
      .toBeGreaterThan(5);
  } finally {
    await fechar();
  }
});

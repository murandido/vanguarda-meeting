// Gate da Fase 0: dois usuários (contextos separados) no mesmo quadro
// enxergam criação, movimento e persistência dos post-its via Loro + WS.
import { expect, test } from "@playwright/test";
import { criarPostit, doisUsuarios, esperaPostit, pixelsColoridos, quadroUnico } from "./canvas";

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
    await criarPostit(a, 400, 300);
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

    // Conta pixels coloridos numa caixa logo abaixo/à direita do ponteiro,
    // onde o triângulo do cursor é desenhado (x..x+12, y..y+12). A grade de
    // pontos do fundo é cinza e não conta.
    const pixelsDesenhados = () => pixelsColoridos(b, x, y);

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

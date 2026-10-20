// CA-01 — canvas infinito: pan, zoom em torno do ponteiro, atalhos de teclado,
// câmeras independentes por usuário e fluidez com 1.000 post-its.
//
// Viewport 1280x800 com DPR 1 (ver playwright.config.ts); a toolbar fixa ocupa
// ~40px no topo, então as sondas ficam abaixo disso. Cada post-it criado por
// duplo clique mede 120x80 e fica centrado no ponto clicado.
import { expect, test, type Page } from "@playwright/test";
import { abrirQuadro, doisUsuarios, esperaPostit, pixelsColoridos, quadroUnico, zoom } from "./canvas";

/** Ctrl + roda `vezes` vezes no ponto (x, y): negativo aproxima. */
async function ctrlRoda(page: Page, x: number, y: number, deltaY: number, vezes: number) {
  await page.mouse.move(x, y);
  await page.keyboard.down("Control");
  for (let i = 0; i < vezes; i++) await page.mouse.wheel(0, deltaY);
  await page.keyboard.up("Control");
}

/** Rolagem sem modificador (trackpad com dois dedos) no ponto (x, y). */
async function rola(page: Page, x: number, y: number, dx: number, dy: number) {
  await page.mouse.move(x, y);
  await page.mouse.wheel(dx, dy);
}

test("pan muda o que está sob um ponto fixo da tela", async ({ page }) => {
  await abrirQuadro(page, quadroUnico("pan"));

  // P1 ocupa x 340..460, y 260..340; P2 ocupa x 740..860, y 260..340.
  await page.mouse.dblclick(400, 300);
  await page.mouse.dblclick(800, 300);
  await esperaPostit(page, 440, 275).toBe(true);
  await esperaPostit(page, 840, 275).toBe(true);

  // Rolagem (200, 100): o conteúdo vai 200px para a esquerda e 100px para cima.
  await rola(page, 640, 500, 200, 100);
  await esperaPostit(page, 440, 275).toBe(false);
  await esperaPostit(page, 240, 175).toBe(true);
  await esperaPostit(page, 640, 175).toBe(true);

  // Espaço + arraste começando EM CIMA de P1: move a vista (os dois juntos),
  // não o post-it.
  await page.mouse.move(200, 200);
  await page.keyboard.down("Space");
  await page.mouse.down();
  await page.mouse.move(300, 300, { steps: 5 });
  await page.mouse.up();
  await page.keyboard.up("Space");
  await esperaPostit(page, 340, 275).toBe(true);
  await esperaPostit(page, 740, 275).toBe(true);

  // Botão do meio arrastando no vazio: volta 100px.
  await page.mouse.move(1000, 600);
  await page.mouse.down({ button: "middle" });
  await page.mouse.move(900, 500, { steps: 5 });
  await page.mouse.up({ button: "middle" });
  await esperaPostit(page, 240, 175).toBe(true);
  await esperaPostit(page, 640, 175).toBe(true);
  await expect(zoom(page)).toHaveText("100%");

  // Com a vista deslocada, arrastar P1 continua movendo só P1 (mundo).
  await page.mouse.move(200, 200);
  await page.mouse.down();
  await page.mouse.move(200, 400, { steps: 5 });
  await page.mouse.up();
  await esperaPostit(page, 240, 375).toBe(true);
  await esperaPostit(page, 240, 175).toBe(false);
  await esperaPostit(page, 640, 175).toBe(true);
});

test("zoom mantém o post-it sob o ponteiro e respeita os atalhos", async ({ page }) => {
  await abrirQuadro(page, quadroUnico("zoom"));
  await expect(zoom(page)).toHaveText("100%");

  // Post-it em x 340..460, y 260..340; ponteiro fora do centro dele.
  await page.mouse.dblclick(400, 300);
  await esperaPostit(page, 420, 310).toBe(true);
  await esperaPostit(page, 500, 370).toBe(false);

  // 4 passos de Ctrl+roda (cada um limitado a e^0,2): 2,2255x ≈ 223%.
  // Com âncora em (420, 310), o post-it vai para x ~242..509, y ~199..377.
  await ctrlRoda(page, 420, 310, -100, 4);
  await expect(zoom(page)).toHaveText("223%");
  await esperaPostit(page, 420, 310).toBe(true);
  await esperaPostit(page, 500, 370).toBe(true);
  await esperaPostit(page, 250, 205).toBe(true);

  // 8 passos para trás: 2,2255 * e^-1,6 ≈ 45%. O ponto sob o ponteiro segue
  // sendo o mesmo post-it, agora pequeno (x ~384..438, y ~288..323).
  await ctrlRoda(page, 420, 310, 100, 8);
  await expect(zoom(page)).toHaveText("45%");
  await esperaPostit(page, 420, 310).toBe(true);
  await esperaPostit(page, 440, 275).toBe(false);

  // Zoom mínimo é 10%.
  await ctrlRoda(page, 420, 310, 100, 20);
  await expect(zoom(page)).toHaveText("10%");

  // Shift+0 volta para 100%.
  await page.keyboard.press("Shift+Digit0");
  await expect(zoom(page)).toHaveText("100%");

  // Shift+1 enquadra o conteúdo: um post-it só bate no zoom máximo (400%),
  // centrado na tela (x 400..880, y 240..560).
  await page.keyboard.press("Shift+Digit1");
  await expect(zoom(page)).toHaveText("400%");
  await esperaPostit(page, 640, 400).toBe(true);
  await esperaPostit(page, 440, 300).toBe(true);
  await esperaPostit(page, 380, 300).toBe(false);

  // Ctrl+0 também volta para 100%, mantendo o centro da tela.
  await page.keyboard.press("Control+Digit0");
  await expect(zoom(page)).toHaveText("100%");
  await esperaPostit(page, 640, 400).toBe(true);
  await esperaPostit(page, 760, 400).toBe(false);
});

test("dois usuários com câmeras diferentes veem o post-it no mesmo ponto do mundo", async ({ browser }) => {
  const { a, b, fechar } = await doisUsuarios(browser, quadroUnico("camera"));
  try {
    // A desloca a vista: canto superior esquerdo de A = mundo (500, 300).
    await rola(a, 640, 500, 500, 300);
    // B desloca de outro jeito: canto de B = mundo (200, 100).
    await rola(b, 640, 500, 200, 100);

    // A cria em tela (400, 300) = mundo (900, 600): x 840..960, y 560..640.
    await a.mouse.dblclick(400, 300);
    await esperaPostit(a, 440, 275).toBe(true);

    // Em B o mesmo post-it aparece em tela x 640..760, y 460..540.
    await esperaPostit(b, 740, 475).toBe(true);
    await esperaPostit(b, 440, 275).toBe(false);

    // Presença em coordenadas do mundo: A em tela (100, 500) = mundo
    // (600, 800), que em B é a tela (400, 700).
    let alterna = 0;
    await expect
      .poll(
        async () => {
          alterna ^= 1;
          await a.mouse.move(100 - alterna, 500);
          await a.waitForTimeout(50);
          await a.mouse.move(100, 500);
          return pixelsColoridos(b, 400, 700);
        },
        { timeout: 5_000 },
      )
      .toBeGreaterThan(5);
    expect(await pixelsColoridos(b, 100, 500)).toBe(0);
  } finally {
    await fechar();
  }
});

test("1.000 post-its: enquadrar e navegar mantém a fluidez", async ({ page }) => {
  const quadro = quadroUnico("mil");
  await abrirQuadro(page, quadro);

  // Gera 1.000 post-its (grade 50x20, passo 160x120) com o próprio módulo
  // WASM e envia por um segundo WebSocket da mesma sessão, como faria outro
  // participante — sem ganchos de teste no código de produção.
  await page.evaluate(async (id) => {
    const caminho = "/pkg/canvas.js";
    const { Board } = await import(caminho);
    const outro = new Board();
    for (let i = 0; i < 1000; i++) outro.addRect(`mil-${i}`, (i % 50) * 160, Math.floor(i / 50) * 120, 120, 80);
    const update: Uint8Array<ArrayBuffer> = outro.takePendingUpdate();
    const ws = new WebSocket(`ws://${location.host}/ws/${id}`);
    ws.binaryType = "arraybuffer";
    await new Promise((ok) => (ws.onopen = ok));
    ws.send(update);
    // Espera o eco do snapshot inicial chegar antes de fechar: o envio já saiu.
    await new Promise((ok) => setTimeout(ok, 200));
    ws.close();
  }, quadro);

  // Post-it 52 = coluna 2, linha 1: x 320..440, y 120..200 em 100%.
  await esperaPostit(page, 380, 160).toBe(true);

  // Shift+1: tudo (7960x2360) cabe na tela com zoom ~15%, centrado em
  // (3980, 1180). O post-it da coluna 24, linha 9 fica em x ~619..637, y ~385..397.
  await page.keyboard.press("Shift+Digit1");
  await expect(zoom(page)).toHaveText("15%");
  await esperaPostit(page, 628, 391).toBe(true);

  // Mede o intervalo entre frames enquanto a vista é arrastada (uma rolagem por
  // frame) com os 1.000 post-its visíveis — o pior caso do desenho.
  const intervalos = await page.evaluate(async () => {
    const canvas = document.getElementById("board")!;
    const tempos: number[] = [];
    await new Promise<void>((fim) => {
      const passo = (t: number) => {
        tempos.push(t);
        const dx = tempos.length % 2 ? 7 : -7;
        canvas.dispatchEvent(
          new WheelEvent("wheel", { deltaX: dx, deltaY: 3, clientX: 640, clientY: 400, bubbles: true, cancelable: true }),
        );
        if (tempos.length < 121) requestAnimationFrame(passo);
        else fim();
      };
      requestAnimationFrame(passo);
    });
    return tempos.slice(1).map((t, i) => t - tempos[i]);
  });
  const ordenados = [...intervalos].sort((x, y) => x - y);
  const mediana = ordenados[Math.floor(ordenados.length / 2)];
  const media = intervalos.reduce((s, x) => s + x, 0) / intervalos.length;
  const fps = 1000 / media;
  test.info().annotations.push({
    type: "desempenho",
    description: `1.000 post-its visíveis: ${fps.toFixed(1)} fps médios, mediana ${mediana.toFixed(1)} ms/frame`,
  });
  console.log(`[CA-01] ${fps.toFixed(1)} fps médios; mediana ${mediana.toFixed(1)} ms/frame`);
  // Piso folgado para máquinas de CI sem GPU; localmente o esperado é ~60 fps.
  expect(mediana).toBeLessThan(1000 / 30);
});

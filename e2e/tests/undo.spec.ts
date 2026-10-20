// CA-08 — desfazer/refazer por usuário: Ctrl+Z de A desfaz só o que A fez,
// sincroniza para B como qualquer edição, e Ctrl+Shift+Z / Ctrl+Y refazem.
//
// Post-its criados por duplo clique medem 120x80 centrados no ponto clicado.
// As sondas ficam acima/à direita do ponto clicado para não cair sob o
// cursor remoto desenhado no canvas do outro usuário.
import { test } from "@playwright/test";
import { doisUsuarios, esperaPostit, quadroUnico } from "./canvas";

test("Ctrl+Z desfaz só as edições do próprio usuário, nos dois lados", async ({ browser }) => {
  const { a, b, fechar } = await doisUsuarios(browser, quadroUnico("undo"));
  try {
    // P_A criado em (400,300): x 340..460, y 260..340.
    const ORIGEM_A = [440, 275] as const;
    // Depois do arraste para (700,500): x 640..760, y 460..540.
    const DESTINO_A = [740, 475] as const;
    // P_B criado por B em (400,600): x 340..460, y 560..640.
    const B_CRIADO = [440, 575] as const;
    // P_B arrastado por B para (1000,300): x 940..1060, y 260..340.
    const B_MOVIDO = [1040, 275] as const;

    await a.mouse.dblclick(400, 300);
    await esperaPostit(b, ...ORIGEM_A).toBe(true);

    // A arrasta o post-it em vários passos: um único passo de desfazer.
    await a.mouse.move(400, 300);
    await a.mouse.down();
    await a.mouse.move(700, 500, { steps: 10 });
    await a.mouse.up();
    await esperaPostit(b, ...DESTINO_A).toBe(true);

    // B cria e move o próprio post-it depois das edições de A.
    await b.mouse.dblclick(400, 600);
    await b.mouse.move(400, 600);
    await b.mouse.down();
    await b.mouse.move(1000, 300, { steps: 5 });
    await b.mouse.up();
    await esperaPostit(a, ...B_MOVIDO).toBe(true);

    // Ctrl+Z em A: o arraste inteiro volta, nos dois lados; o de B fica.
    await a.keyboard.press("Control+KeyZ");
    for (const p of [a, b]) {
      await esperaPostit(p, ...ORIGEM_A).toBe(true);
      await esperaPostit(p, ...DESTINO_A).toBe(false);
      await esperaPostit(p, ...B_MOVIDO).toBe(true);
    }

    // Segundo Ctrl+Z desfaz a criação; o post-it de B continua onde B deixou.
    await a.keyboard.press("Control+KeyZ");
    for (const p of [a, b]) {
      await esperaPostit(p, ...ORIGEM_A).toBe(false);
      await esperaPostit(p, ...B_MOVIDO).toBe(true);
      await esperaPostit(p, ...B_CRIADO).toBe(false);
    }

    // Nada mais de A para desfazer: outro Ctrl+Z não mexe no que é de B.
    await a.keyboard.press("Control+KeyZ");
    await a.waitForTimeout(200);
    await esperaPostit(b, ...B_MOVIDO).toBe(true);

    // Refazer: Ctrl+Shift+Z recria, Ctrl+Y refaz o arraste.
    await a.keyboard.press("Control+Shift+KeyZ");
    await esperaPostit(b, ...ORIGEM_A).toBe(true);
    await a.keyboard.press("Control+KeyY");
    for (const p of [a, b]) {
      await esperaPostit(p, ...DESTINO_A).toBe(true);
      await esperaPostit(p, ...ORIGEM_A).toBe(false);
      await esperaPostit(p, ...B_MOVIDO).toBe(true);
    }
  } finally {
    await fechar();
  }
});

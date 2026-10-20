// CA-04 — stickers: stamp from the tray, move, scale and rotate, all seen by
// the other user.
//
// The board background is transparent and the grid dots translucent, so a
// sticker is detected by its opaque pixels — whatever emoji font the machine
// has (a missing glyph still draws an opaque box).
import { expect, test, type Page } from "@playwright/test";
import { contaPixels, doisUsuarios, mascara, pixelsColoridos, quadroUnico } from "./canvas";

/** Box of side `lado` centered on (x, y). */
const caixa = (x: number, y: number, lado: number) => [x - lado / 2, y - lado / 2, lado, lado] as const;

/**
 * Parks A's pointer at (1100, 700) and waits until B draws A's cursor there,
 * so the cursor no longer covers the spots under test. board.js throttles
 * cursor messages, so the pointer wiggles 1px until one gets through.
 */
async function afastaCursor(a: Page, b: Page) {
  let alterna = 0;
  await expect
    .poll(
      async () => {
        alterna ^= 1;
        await a.mouse.move(1100 - alterna, 700);
        await a.waitForTimeout(50);
        await a.mouse.move(1100, 700);
        return pixelsColoridos(b, 1100, 700);
      },
      { timeout: 5_000 },
    )
    .toBeGreaterThan(5);
}

test("figurinha: carimbar, mover, escalar, girar e apagar", async ({ browser }) => {
  const { a, b, fechar } = await doisUsuarios(browser, quadroUnico("figurinha"));
  const opacos = (x: number, y: number, lado: number) => contaPixels(b, caixa(x, y, lado), "opaco");
  try {
    // Pick a sticker from the tray, then click the board to stamp it.
    const tray = a.getByRole("toolbar", { name: "Figurinhas" });
    await expect(tray).toBeHidden();
    await a.getByRole("button", { name: /Figurinhas/ }).click();
    await expect(tray).toBeVisible();
    await expect(tray.getByRole("button")).toHaveCount(24);
    await tray.getByRole("button", { name: "Figurinha 👍" }).click();
    await expect(tray).toBeHidden();
    await a.mouse.click(400, 300);
    await afastaCursor(a, b);
    await expect.poll(() => opacos(400, 300, 64), { timeout: 5_000 }).toBeGreaterThan(100);

    // Drag it to (700, 500).
    await a.mouse.move(400, 300);
    await a.mouse.down();
    await a.mouse.move(700, 500, { steps: 8 });
    await a.mouse.up();
    await afastaCursor(a, b);
    await expect.poll(() => opacos(700, 500, 64), { timeout: 5_000 }).toBeGreaterThan(100);
    expect(await opacos(400, 300, 64)).toBe(0);

    // Scale handle at the bottom-right corner (732, 532): dragging it to
    // (764, 564) doubles the sticker, which now reaches past the old box.
    const antes = await opacos(700, 500, 160);
    await a.mouse.move(732, 532);
    await a.mouse.down();
    await a.mouse.move(764, 564, { steps: 4 });
    await a.mouse.up();
    await afastaCursor(a, b);
    await expect.poll(() => opacos(700, 500, 160), { timeout: 5_000 }).toBeGreaterThan(antes * 2);

    // ] rotates 15° clockwise, [ back: six steps change the drawing and six
    // steps back restore it exactly.
    const regiao = caixa(700, 500, 180);
    const original = await mascara(b, regiao);
    for (let i = 0; i < 6; i++) await a.keyboard.press("BracketRight");
    await expect.poll(() => mascara(b, regiao), { timeout: 5_000 }).not.toBe(original);
    for (let i = 0; i < 6; i++) await a.keyboard.press("BracketLeft");
    await expect.poll(() => mascara(b, regiao), { timeout: 5_000 }).toBe(original);

    // Backspace deletes the selected sticker.
    await a.keyboard.press("Backspace");
    await expect.poll(() => opacos(700, 500, 160), { timeout: 5_000 }).toBe(0);
  } finally {
    await fechar();
  }
});

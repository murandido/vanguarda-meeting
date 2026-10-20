// CA-02 — post-its: create with the keyboard, edit text inline, recolor,
// resize and delete, all seen live by the other user.
//
// Viewport 1280x800 at DPR 1 (see playwright.config.ts). A new post-it is
// 120x80 centered on the pointer: created at (400, 300) it spans x 340..460,
// y 260..340.
import { expect, test } from "@playwright/test";
import { CORES_POSTIT, contaPixels, doisUsuarios, ehCor, esperaPostit, pixel, quadroUnico } from "./canvas";

/** First text line of the post-it created at (400, 300). */
const LINHA_1 = [345, 262, 50, 30] as const;

test("post-it criado pelo teclado recebe texto que o outro usuário vê", async ({ browser }) => {
  const { a, b, fechar } = await doisUsuarios(browser, quadroUnico("texto"));
  try {
    // N creates a post-it under the pointer and opens its editor.
    await a.mouse.move(400, 300);
    await a.keyboard.press("n");
    const editorA = a.locator("#postit-editor");
    await expect(editorA).toBeFocused();
    await esperaPostit(b, 440, 275).toBe(true);
    expect(await contaPixels(b, LINHA_1, "escuro")).toBe(0);

    await a.keyboard.type("Olá mundo");
    // Shift+Enter is a line break; Enter commits.
    await a.keyboard.press("Shift+Enter");
    await a.keyboard.type("!");
    await expect(editorA).toHaveValue("Olá mundo\n!");
    await a.keyboard.press("Enter");
    await expect(editorA).toBeHidden();

    // B draws the text on the canvas...
    await expect.poll(() => contaPixels(b, LINHA_1, "escuro"), { timeout: 5_000 }).toBeGreaterThan(20);
    // ...and gets the exact text when opening the same post-it.
    await b.mouse.dblclick(440, 330);
    const editorB = b.locator("#postit-editor");
    await expect(editorB).toBeFocused();
    await expect(editorB).toHaveValue("Olá mundo\n!");

    // B appends; A sees it in its own editor (text merged in the CRDT).
    await b.keyboard.press("Control+End");
    await b.keyboard.type(" ok");
    await b.keyboard.press("Escape");
    await a.mouse.dblclick(440, 330);
    await expect(editorA).toHaveValue("Olá mundo\n! ok");
    await a.keyboard.press("Escape");

    // The text is part of the saved snapshot.
    await b.reload();
    await expect(b.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
    await expect.poll(() => contaPixels(b, LINHA_1, "escuro"), { timeout: 5_000 }).toBeGreaterThan(20);
  } finally {
    await fechar();
  }
});

test("cor, redimensionamento e exclusão do post-it chegam ao outro usuário", async ({ browser }) => {
  const { a, b, fechar } = await doisUsuarios(browser, quadroUnico("postit"));
  const azul = (page: typeof a, x: number, y: number) =>
    expect.poll(async () => ehCor(await pixel(page, x, y), CORES_POSTIT.blue), { timeout: 5_000 });
  try {
    await a.mouse.dblclick(400, 300);
    await a.keyboard.press("Escape");
    await esperaPostit(b, 440, 275).toBe(true);

    // The selected post-it shows the color toolbar.
    const cores = a.getByRole("toolbar", { name: "Cor do post-it" });
    await expect(cores).toBeVisible();
    await expect(cores.getByRole("button", { name: "Amarelo" })).toHaveAttribute("aria-pressed", "true");
    await cores.getByRole("button", { name: "Azul" }).click();
    await expect(cores.getByRole("button", { name: "Azul" })).toHaveAttribute("aria-pressed", "true");
    await azul(b, 440, 275).toBe(true);
    await esperaPostit(b, 440, 275).toBe(false);

    // Resize handle at the bottom-right corner (460, 340): drag it 100px.
    await a.mouse.move(460, 340);
    await a.mouse.down();
    await a.mouse.move(560, 440, { steps: 8 });
    await a.mouse.up();
    await azul(b, 540, 420).toBe(true);
    await azul(b, 440, 275).toBe(true);

    // Dragging past the top-left corner stops at the minimum size (40x40).
    await a.mouse.move(560, 440);
    await a.mouse.down();
    await a.mouse.move(250, 150, { steps: 8 });
    await a.mouse.up();
    await azul(b, 370, 290).toBe(true);
    await azul(b, 400, 320).toBe(false);

    // Delete removes the selected post-it.
    await a.keyboard.press("Delete");
    await azul(b, 370, 290).toBe(false);
    await expect(cores).toBeHidden();
  } finally {
    await fechar();
  }
});

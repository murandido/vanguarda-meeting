// CA-03 — arrows between shapes: create from an anchor dot, see it on the
// other screen, bend it by moving a shape, switch to straight, delete it;
// arrow tool (A) to a free point.
//
// Viewport 1280x800, DPR 1; each double click creates a 120x80 shape centered
// on the click. Arrows are drawn in saturated blue (orange when selected), so
// they show up in `pixelsColoridos` while the gray dot grid does not.
import { expect, test, type Page } from "@playwright/test";
import { abrirNovoQuadro, doisUsuarios, esperaPostit, pixelsColoridos, quadroUnico } from "./canvas";

/** Waits until there is (or is not) arrow ink in a 6x6 box around (x, y). */
const esperaSeta = (page: Page, x: number, y: number) =>
  expect.poll(async () => (await pixelsColoridos(page, x - 3, y - 3, 6, 6)) > 0, { timeout: 5_000 });

/** Creates a post-it centered on (x, y) and closes its text editor. */
async function criaPostit(page: Page, x: number, y: number) {
  await page.mouse.dblclick(x, y);
  await expect(page.locator("#postit-editor")).toBeFocused();
  await page.keyboard.press("Escape");
}

/** Drags with the left button from one screen point to another. */
async function arrasta(page: Page, from: [number, number], to: [number, number]) {
  await page.mouse.move(...from);
  await page.mouse.down();
  await page.mouse.move(...to, { steps: 10 });
  await page.mouse.up();
}

test("arrow between two shapes syncs, follows moves and can be deleted", async ({ browser }) => {
  const { a, b, fechar } = await doisUsuarios(browser, quadroUnico("setas"));
  try {
    // S1: x 240..360, y 260..340. S2: x 640..760, y 260..340.
    await criaPostit(a, 300, 300);
    await criaPostit(a, 700, 300);
    await esperaPostit(b, 340, 275).toBe(true);
    await esperaPostit(b, 740, 275).toBe(true);
    await esperaSeta(b, 500, 300).toBe(false);

    // Hover S1, then drag its east anchor dot (360, 300) onto S2: the arrow
    // goes straight from S1's east side to S2's west side (640, 300).
    await a.mouse.move(300, 300);
    await arrasta(a, [360, 300], [700, 300]);
    await esperaSeta(a, 500, 300).toBe(true);
    await esperaSeta(b, 500, 300).toBe(true);
    // Dragging an anchor creates an arrow; it does not move the shape.
    await esperaPostit(b, 340, 275).toBe(true);
    await a.keyboard.press("Escape");

    // B drags S2 down to x 640..760, y 560..640. The arrow now enters S2 from
    // the top (700, 560) and bends: its midpoint is near (610, 350).
    await arrasta(b, [700, 300], [700, 600]);
    for (const page of [a, b]) {
      await esperaPostit(page, 740, 575).toBe(true);
      await esperaSeta(page, 610, 350).toBe(true);
      await esperaSeta(page, 630, 300).toBe(false);
    }

    // A selects the arrow on the curve and switches it to straight (R): the
    // line from (360, 300) to (700, 560) passes through (530, 430), and the
    // curve's former path near (600, 343) is empty.
    await a.mouse.click(610, 350);
    await a.keyboard.press("r");
    for (const page of [a, b]) {
      await esperaSeta(page, 530, 430).toBe(true);
      // Next to (not on) A's pointer, which B draws as a colored cursor.
      await esperaSeta(page, 600, 343).toBe(false);
    }

    // Delete removes the (still selected) arrow for both; the shapes stay.
    await a.keyboard.press("Delete");
    for (const page of [a, b]) {
      await esperaSeta(page, 530, 430).toBe(false);
      await esperaPostit(page, 340, 275).toBe(true);
      await esperaPostit(page, 740, 575).toBe(true);
    }

    // Persistence: a new (curved, the default) arrow survives a reload.
    await a.mouse.move(300, 300);
    await arrasta(a, [360, 300], [700, 600]);
    await esperaSeta(b, 600, 343).toBe(true);
    await b.reload();
    await expect(b.locator("#status")).toHaveAttribute("data-state", "online", { timeout: 15_000 });
    await esperaSeta(b, 600, 343).toBe(true);
  } finally {
    await fechar();
  }
});

test("arrow tool connects a shape to a free point", async ({ page }) => {
  await abrirNovoQuadro(page, quadroUnico("seta-livre"));
  const ferramenta = page.locator("#tool-arrow");

  // S1: x 240..360, y 260..340.
  await criaPostit(page, 300, 300);
  await esperaPostit(page, 340, 275).toBe(true);

  // A turns the tool on; dragging from the middle of the shape does not move
  // it but draws an arrow from its bottom side (300, 340) to (300, 600).
  await page.keyboard.press("a");
  await expect(ferramenta).toHaveAttribute("aria-pressed", "true");
  await arrasta(page, [300, 300], [300, 600]);
  await esperaPostit(page, 340, 275).toBe(true);
  await esperaSeta(page, 300, 470).toBe(true);
  // The tool turns itself off after one arrow.
  await expect(ferramenta).toHaveAttribute("aria-pressed", "false");

  // The toolbar button works too; a click without dragging creates nothing.
  await ferramenta.click();
  await expect(ferramenta).toHaveAttribute("aria-pressed", "true");
  await page.mouse.click(300, 300);
  await page.keyboard.press("Escape");
  await expect(ferramenta).toHaveAttribute("aria-pressed", "false");
  await esperaSeta(page, 500, 300).toBe(false);

  // Deleting the shape deletes the arrows attached to it.
  await page.mouse.click(300, 300);
  await page.keyboard.press("Delete");
  await esperaPostit(page, 340, 275).toBe(false);
  await esperaSeta(page, 300, 470).toBe(false);
});

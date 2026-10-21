// Cartazes — arrows between shapes (CA-03).
//
// Geometry, routing and hit-testing live in Rust (crates/canvas/src/
// connectors.rs); this module only turns pointer/keyboard events into board
// calls and draws the flat arrays it gets back. board.js calls the hooks
// returned by `createConnectors` at a few clearly marked points.
//
// Interactions:
// - hover a shape: four anchor dots (N/E/S/W); drag from a dot to another
//   shape (or to empty space) creates an arrow;
// - arrow tool ("Seta" button or A): drag from any shape to another;
// - click an arrow selects it; Delete/Backspace removes it; R switches
//   between curved and straight; Esc cancels/deselects.

/** Saturated colors so the arrows are easy to tell apart (and to probe in E2E). */
export const ARROW_COLOR = "#3b5bdb";
export const SELECTED_COLOR = "#e8590c";

const ANCHOR_RADIUS_PX = 5; // drawn dot
const ANCHOR_HIT_PX = 9; // grab radius
const ARROW_HIT_PX = 6; // tolerance to select an arrow
const MIN_DRAG_PX = 4; // shorter drags are clicks and create nothing
const LINE_WIDTH = 2; // world units
const HEAD_LENGTH = 12; // world units
const HEAD_WIDTH = 9; // world units

/**
 * @param {object} deps
 * @param {HTMLCanvasElement} deps.canvas
 * @param {import("/pkg/canvas.js").Board} deps.board
 * @param {import("/pkg/canvas.js").Camera} deps.camera
 * @param {() => void} deps.commit  send local changes and redraw
 * @param {() => void} deps.changed redraw only
 */
export function createConnectors({ canvas, board, camera, commit, changed }) {
  const stride = board.constructor.connectorStride();
  const toolButton = document.getElementById("tool-arrow");

  let tool = false; // arrow tool active
  let hovered = null; // shape id showing anchor dots
  let selected = null; // selected connector id
  let drag = null; // { from, side, sx, sy, x, y, target: {shape, side} | null }

  function setTool(on) {
    tool = on;
    toolButton?.setAttribute("aria-pressed", String(on));
    canvas.classList.toggle("tool-arrow", on);
  }
  toolButton?.addEventListener("click", () => {
    toolButton.blur();
    setTool(!tool);
  });

  function select(id) {
    if (selected !== id) {
      selected = id;
      changed();
    }
  }

  const world = (e) => [camera.toWorldX(e.offsetX), camera.toWorldY(e.offsetY)];

  /** Anchor dot under the world point, as `{ shape, side }`. */
  function anchorAt(x, y) {
    const a = board.anchorNear(camera, x, y, ANCHOR_HIT_PX);
    if (!a) return null;
    const out = { shape: a.shape, side: a.side };
    a.free();
    return out;
  }

  function shapeAt(x, y) {
    const hit = board.hitTest(x, y);
    if (!hit) return null;
    const id = hit.id;
    hit.free();
    return id;
  }

  /** Where a drag in progress would end: a shape (side or auto) or nothing. */
  function targetAt(x, y, from) {
    const a = anchorAt(x, y);
    if (a && a.shape !== from) return a;
    const id = shapeAt(x, y);
    return id && id !== from ? { shape: id, side: "auto" } : null;
  }

  function startDrag(e, from, side) {
    const [x, y] = world(e);
    drag = { from, side, sx: e.offsetX, sy: e.offsetY, x, y, target: null };
    select(null);
    changed();
  }

  // ---- hooks called by board.js ------------------------------------------

  /** Primary button down (no pan modifier). True if this module took it. */
  function pointerDown(e) {
    const [x, y] = world(e);
    if (tool) {
      const id = shapeAt(x, y);
      if (id) startDrag(e, id, "auto");
      else select(null);
      return true; // the arrow tool never moves shapes
    }
    const a = anchorAt(x, y);
    if (a) {
      startDrag(e, a.shape, a.side);
      return true;
    }
    const hit = board.hitConnector(camera, x, y, ARROW_HIT_PX);
    select(hit ?? null);
    return hit !== undefined;
  }

  /** Pointer moved. True while dragging an arrow (board.js skips its own drag). */
  function pointerMove(e, busy) {
    const [x, y] = world(e);
    if (drag) {
      drag.x = x;
      drag.y = y;
      drag.target = targetAt(x, y, drag.from);
      hovered = drag.target?.shape ?? null;
      changed();
      return true;
    }
    if (!busy) {
      const next = anchorAt(x, y)?.shape ?? shapeAt(x, y);
      if (next !== hovered) {
        hovered = next;
        changed();
      }
    }
    return false;
  }

  /** Pointer released or cancelled: finishes an arrow drag, if any. */
  function pointerUp(e) {
    if (!drag) return;
    const d = drag;
    drag = null;
    changed();
    if (e.type !== "pointerup") return; // pointercancel: drop the arrow
    const moved = Math.hypot(e.offsetX - d.sx, e.offsetY - d.sy) >= MIN_DRAG_PX;
    // Released on the source shape itself: nothing to connect.
    if (!moved || (!d.target && shapeAt(d.x, d.y) === d.from)) return;
    const id = crypto.randomUUID();
    const t = d.target;
    board.addConnector(id, d.from, d.side, t?.shape, t?.side ?? "auto", d.x, d.y);
    select(id);
    setTool(false);
    commit();
  }

  /** Key pressed outside text fields. True if handled. */
  function keyDown(e) {
    if (e.ctrlKey || e.metaKey || e.altKey) return false;
    if (e.code === "KeyA" && !e.shiftKey) {
      setTool(!tool);
      return true;
    }
    if (e.code === "Escape" && (tool || drag || selected)) {
      setTool(false);
      drag = null;
      select(null);
      changed();
      return true;
    }
    if (!selected) return false;
    if (e.code === "Delete" || e.code === "Backspace") {
      board.removeConnector(selected);
      select(null);
      commit();
      return true;
    }
    if (e.code === "KeyR" && !e.shiftKey) {
      board.toggleConnectorRoute(selected);
      commit();
      return true;
    }
    return false;
  }

  // ---- drawing -------------------------------------------------------------

  function path(ctx, g, i) {
    // Stop the line at the base of the arrowhead so its end hides under it.
    const dx = g[i + 8];
    const dy = g[i + 9];
    const ex = g[i + 6] - dx * HEAD_LENGTH * 0.8;
    const ey = g[i + 7] - dy * HEAD_LENGTH * 0.8;
    ctx.moveTo(g[i], g[i + 1]);
    ctx.bezierCurveTo(g[i + 2], g[i + 3], g[i + 4], g[i + 5], ex, ey);
  }

  function head(ctx, g, i) {
    const tx = g[i + 6];
    const ty = g[i + 7];
    const dx = g[i + 8];
    const dy = g[i + 9];
    const bx = tx - dx * HEAD_LENGTH;
    const by = ty - dy * HEAD_LENGTH;
    const w = HEAD_WIDTH / 2;
    ctx.moveTo(tx, ty);
    ctx.lineTo(bx - dy * w, by + dx * w);
    ctx.lineTo(bx + dy * w, by - dx * w);
    ctx.closePath();
  }

  /** Draws every arrow in `g` (flat, `stride` numbers each) in one path. */
  function drawAll(ctx, g, color, width) {
    if (g.length === 0) return;
    ctx.strokeStyle = color;
    ctx.fillStyle = color;
    ctx.lineWidth = width;
    ctx.beginPath();
    for (let i = 0; i < g.length; i += stride) path(ctx, g, i);
    ctx.stroke();
    ctx.beginPath();
    for (let i = 0; i < g.length; i += stride) head(ctx, g, i);
    ctx.fill();
  }

  /**
   * Draws arrows, selection and drag preview. Called with the world
   * transform set; `w`/`h` are the canvas size in CSS pixels.
   */
  function draw(ctx, w, h, zoom) {
    const width = Math.max(LINE_WIDTH, 1 / zoom);
    drawAll(ctx, board.visibleConnectors(camera, w, h), ARROW_COLOR, width);
    if (selected) {
      const g = board.connectorGeometry(selected);
      // The selected arrow was deleted (here or remotely): forget it.
      if (g.length === 0) selected = null;
      else drawAll(ctx, g, SELECTED_COLOR, width * 1.75);
    }
    if (drag) {
      const t = drag.target;
      const g = board.previewConnector(drag.from, drag.side, t?.shape, t?.side ?? "auto", drag.x, drag.y);
      ctx.globalAlpha = 0.6;
      drawAll(ctx, g, ARROW_COLOR, width);
      ctx.globalAlpha = 1;
    }
    const dots = [];
    if (hovered) dots.push(hovered);
    if (drag && drag.from !== hovered) dots.push(drag.from);
    for (const id of dots) drawAnchors(ctx, board.anchorPoints(id), zoom);
  }

  function drawAnchors(ctx, pts, zoom) {
    if (pts.length === 0) return;
    const r = ANCHOR_RADIUS_PX / zoom;
    ctx.lineWidth = 1.5 / zoom;
    ctx.strokeStyle = ARROW_COLOR;
    ctx.fillStyle = "#ffffff";
    ctx.beginPath();
    for (let i = 0; i < pts.length; i += 2) {
      ctx.moveTo(pts[i] + r, pts[i + 1]);
      ctx.arc(pts[i], pts[i + 1], r, 0, Math.PI * 2);
    }
    ctx.fill();
    ctx.stroke();
  }

  return { pointerDown, pointerMove, pointerUp, keyDown, draw };
}

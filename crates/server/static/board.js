// Cartazes — post-its and stickers synced through Loro on an infinite board.
// State (document and camera) lives in WASM; this file only handles events,
// networking and drawing. Coordinates: "screen" = CSS pixels of the canvas;
// "world" = board coordinates, the same for everyone. The camera (pan/zoom)
// is local to each participant.
import init, { Board, Camera, colorNames, shapeLimits, stickerPalette } from "/pkg/canvas.js";

await init();

const canvas = document.getElementById("board");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const zoomEl = document.getElementById("zoom");
const stickersBtn = document.getElementById("stickers");
const trayEl = document.getElementById("sticker-tray");
const colorsEl = document.getElementById("postit-colors");
const editor = document.getElementById("postit-editor");
const boardId = canvas.dataset.board;

// Session login; the server ignores whatever name the client sends.
const user = canvas.dataset.user;

const board = new Board();
const camera = new Camera();
const cursors = new Map(); // user -> { x, y, seen } (world)
let ws;
let dirty = true;

// ---- shape model (mirrors crates/canvas/src/shape.rs) ---------------------

/** Values per shape in `visibleShapes()` / `shapeRecord()`. */
const RECORD = 8;
const POSTIT = 0;
const STICKER = 1;
const [POSTIT_MIN, , STICKER_SIZE, STICKER_MIN_SCALE, STICKER_MAX_SCALE, MAX_TEXT_CHARS] = shapeLimits();
const COLOR_NAMES = colorNames();
const STICKERS = stickerPalette();

/** Post-it fills, by color name (same palette as the swatches in app.css). */
const POSTIT_COLORS = {
  yellow: "#ffe066",
  orange: "#ffc078",
  pink: "#faa2c1",
  purple: "#d0bfff",
  blue: "#a5d8ff",
  green: "#b2f2bb",
};
const COLOR_LABELS = {
  yellow: "Amarelo",
  orange: "Laranja",
  pink: "Rosa",
  purple: "Roxo",
  blue: "Azul",
  green: "Verde",
};
const POSTIT_FILLS = COLOR_NAMES.map((name) => POSTIT_COLORS[name]);

/** New post-its: size in world units, centered on the point. */
const NEW_POSTIT_W = 120;
const NEW_POSTIT_H = 80;

const SELECTION = "#0d99ff";
const HANDLE_PX = 8;
const ROTATE_HANDLE_GAP_PX = 22;
const ROTATE_STEP = Math.PI / 12; // 15°

/**
 * Ids sort by creation time (base-36 milliseconds, fixed width until 2059),
 * so the draw order — sorted by id — puts newer shapes on top.
 */
function newId() {
  return `${Date.now().toString(36).padStart(9, "0")}-${crypto.randomUUID().slice(0, 8)}`;
}

function colorFor(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return `hsl(${h} 70% 45%)`;
}

/** Something visible changed: redraw on the next frame. */
function changed() {
  dirty = true;
}

/** The camera changed: redraw, update the toolbar zoom and the remote cursor. */
function cameraChanged() {
  changed();
  zoomEl.textContent = `${Math.round(camera.zoom * 100)}%`;
  // A pointer at rest now points at another spot of the world.
  if (pointer.inside) sendCursor(pointer.x, pointer.y);
}

/** A local edit: send it and redraw. */
function edited() {
  flush();
  changed();
}

// ---- network --------------------------------------------------------------

function connect() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  ws = new WebSocket(`${proto}//${location.host}/ws/${boardId}`);
  ws.binaryType = "arraybuffer";
  ws.onopen = () => {
    setStatus("online", "online");
    flush();
  };
  ws.onmessage = (ev) => {
    if (ev.data instanceof ArrayBuffer) {
      board.import(new Uint8Array(ev.data));
      remoteEdit();
    } else {
      const msg = JSON.parse(ev.data);
      if (msg.type === "cursor") cursors.set(msg.user, { x: msg.x, y: msg.y, seen: performance.now() });
      if (msg.type === "leave") cursors.delete(msg.user);
    }
    changed();
  };
  ws.onclose = () => {
    setStatus("offline", "reconectando…");
    setTimeout(connect, 1000);
  };
}

function setStatus(state, text) {
  statusEl.dataset.state = state;
  statusEl.textContent = text;
}

function flush() {
  if (ws?.readyState !== WebSocket.OPEN) return;
  const update = board.takePendingUpdate();
  if (update) ws.send(update);
}

let lastCursorSent = 0;
/** Sends the cursor in WORLD coordinates, from a screen point. */
function sendCursor(sx, sy) {
  const now = performance.now();
  if (ws?.readyState !== WebSocket.OPEN || now - lastCursorSent < 33) return;
  lastCursorSent = now;
  ws.send(JSON.stringify({ type: "cursor", user, x: camera.toWorldX(sx), y: camera.toWorldY(sy) }));
}

/** After a remote update: drop a selection that was deleted, sync the editor. */
function remoteEdit() {
  if (selected && !board.shapeRecord(selected)) select(null);
  if (editing) syncEditorFromBoard();
}

// ---- text layout ------------------------------------------------------------
//
// Post-it text wraps at word boundaries and shrinks to fit the box, down to a
// minimum size. Layouts are cached by (text version, w, h): the text itself
// only crosses from WASM when a version is new.

const FONT_FAMILY = "system-ui, sans-serif";
const TEXT_PAD = 8;
const LINE_HEIGHT = 1.25;
const MAX_FONT = 24;
const MIN_FONT = 6;
const TEXT_COLOR = "#1f1f1f";
/** Below this on-screen size the text is unreadable: skip drawing it. */
const MIN_READABLE_PX = 3;
const CACHE_LIMIT = 4000;

const measureCtx = document.createElement("canvas").getContext("2d");
measureCtx.font = `100px ${FONT_FAMILY}`;
/** Width of a string at 100px; width scales linearly with font size. */
const widths = new Map();
function width100(s) {
  let w = widths.get(s);
  if (w === undefined) {
    w = measureCtx.measureText(s).width;
    if (widths.size > CACHE_LIMIT * 4) widths.clear();
    widths.set(s, w);
  }
  return w;
}

/** Greedy word wrap of `text` into lines at most `maxW` wide at `size`. */
function wrap(text, maxW, size) {
  const k = size / 100;
  const lines = [];
  for (const paragraph of text.split("\n")) {
    let line = "";
    for (const word of paragraph.split(/(?<= )/)) {
      const candidate = line + word;
      if (line === "" || width100(candidate.trimEnd()) * k <= maxW) {
        line = candidate;
      } else {
        lines.push(line.trimEnd());
        line = word;
      }
      // A single word wider than the box breaks between characters.
      while (width100(line.trimEnd()) * k > maxW && [...line.trimEnd()].length > 1) {
        const chars = [...line];
        let n = chars.length - 1;
        while (n > 1 && width100(chars.slice(0, n).join("")) * k > maxW) n--;
        lines.push(chars.slice(0, n).join(""));
        line = chars.slice(n).join("");
      }
    }
    lines.push(line.trimEnd());
  }
  return lines;
}

/** Largest font size (MIN_FONT..MAX_FONT) at which `text` fits the box. */
function layoutText(text, w, h) {
  const maxW = Math.max(1, w - 2 * TEXT_PAD);
  const maxH = Math.max(1, h - 2 * TEXT_PAD);
  let size = MAX_FONT;
  for (;;) {
    const lines = wrap(text, maxW, size);
    if (lines.length * size * LINE_HEIGHT <= maxH || size <= MIN_FONT) return { size, lines };
    size = Math.max(MIN_FONT, size * 0.9);
  }
}

const texts = new Map(); // text version -> string
const layouts = new Map(); // `${version}|${w}|${h}` -> { size, lines }

function cachedLayout(index, version, w, h) {
  const key = `${version}|${w}|${h}`;
  let layout = layouts.get(key);
  if (!layout) {
    let text = texts.get(version);
    if (text === undefined) {
      text = board.textAt(index);
      if (texts.size > CACHE_LIMIT) texts.clear();
      texts.set(version, text);
    }
    layout = layoutText(text, w, h);
    if (layouts.size > CACHE_LIMIT) layouts.clear();
    layouts.set(key, layout);
  }
  return layout;
}

// ---- selection, color toolbar and text editor --------------------------------

let selected = null; // shape id
let editing = null; // post-it id whose text is in the editor

function select(id) {
  if (editing && editing !== id) stopEditing();
  selected = id;
  changed();
}

for (const name of COLOR_NAMES) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = "swatch";
  b.dataset.color = name;
  b.title = COLOR_LABELS[name];
  b.setAttribute("aria-label", COLOR_LABELS[name]);
  b.setAttribute("aria-pressed", "false");
  b.addEventListener("click", () => {
    if (!selected) return;
    board.setColor(selected, name);
    edited();
  });
  colorsEl.append(b);
}
// Clicking a swatch must not steal focus from the text editor.
colorsEl.addEventListener("pointerdown", (e) => e.preventDefault());

function startEditing(id) {
  const rec = board.shapeRecord(id);
  if (!rec || rec[1] !== POSTIT) return;
  select(id);
  editing = id;
  editor.maxLength = MAX_TEXT_CHARS;
  editor.value = board.text(id);
  editor.hidden = false;
  placeEditor(rec);
  editor.focus();
  editor.setSelectionRange(editor.value.length, editor.value.length);
  changed();
}

function stopEditing() {
  if (!editing) return;
  editing = null;
  editor.hidden = true;
  if (document.activeElement === editor) editor.blur();
  changed();
}

/** Sizes the editor in world units and lets the camera transform scale it. */
function placeEditor(rec) {
  const [, , x, y, w, h] = rec;
  const zoom = camera.zoom;
  const s = editor.style;
  s.width = `${w}px`;
  s.height = `${h}px`;
  s.fontSize = `${layoutText(editor.value, w, h).size}px`;
  s.transform = `translate(${(x - camera.x) * zoom}px, ${(y - camera.y) * zoom}px) scale(${zoom})`;
}

/** A remote edit changed the text under the editor: keep the caret in place. */
function syncEditorFromBoard() {
  let text;
  try {
    text = board.text(editing);
  } catch {
    stopEditing(); // deleted by someone else
    return;
  }
  const old = editor.value;
  if (text === old) return;
  // Single splice old -> text (UTF-16 units, like the selection offsets).
  let p = 0;
  while (p < old.length && p < text.length && old[p] === text[p]) p++;
  let s = 0;
  while (s < old.length - p && s < text.length - p && old[old.length - 1 - s] === text[text.length - 1 - s]) s++;
  const delta = text.length - old.length;
  const shift = (i) => (i <= p ? i : i >= old.length - s ? i + delta : p + (text.length - s - p));
  const [a, b] = [shift(editor.selectionStart), shift(editor.selectionEnd)];
  editor.value = text;
  editor.setSelectionRange(a, b);
}

editor.addEventListener("input", () => {
  if (!editing) return;
  board.setText(editing, editor.value);
  edited();
});
editor.addEventListener("keydown", (e) => {
  // Enter commits; Shift+Enter is a new line.
  if (e.key === "Escape" || (e.key === "Enter" && !e.shiftKey && !e.isComposing)) {
    e.preventDefault();
    stopEditing();
  }
});
editor.addEventListener("blur", stopEditing);
editor.addEventListener("wheel", onWheel, { passive: false });

// ---- sticker tray -------------------------------------------------------------

let stamp = null; // emoji waiting for a click on the board

for (const emoji of STICKERS) {
  const b = document.createElement("button");
  b.type = "button";
  b.textContent = emoji;
  b.title = emoji;
  b.setAttribute("aria-label", `Figurinha ${emoji}`);
  b.addEventListener("click", () => {
    setTray(false);
    setStamp(emoji);
  });
  trayEl.append(b);
}

function setTray(open) {
  trayEl.hidden = !open;
  stickersBtn.setAttribute("aria-expanded", String(open));
}

function setStamp(emoji) {
  stamp = emoji;
  canvas.classList.toggle("stamping", stamp !== null);
}

stickersBtn.addEventListener("click", () => {
  setStamp(null);
  setTray(trayEl.hidden);
});

// ---- interaction --------------------------------------------------------------
//
// Pan: space + drag, middle button, or two-finger scroll (wheel without Ctrl).
// Zoom: Ctrl/Cmd + wheel or pinch (wheel with ctrlKey), around the pointer.
// Keys: Shift+0 / Ctrl+0 = 100%, Shift+1 = fit all, N = new post-it, Enter =
// edit, Delete/Backspace = delete, [ ] = rotate sticker, Esc = cancel/deselect.

const pointer = { x: 0, y: 0, inside: false };
let spaceHeld = false;
// { kind: "pan", lastX, lastY } | { kind: "drag", id, dx, dy }
// | { kind: "resize", id, x, y } | { kind: "scale", id, x, y, rotation }
// | { kind: "rotate", id, x, y, scale }
let gesture = null;

function updateCursorStyle() {
  canvas.style.cursor = gesture?.kind === "pan" ? "grabbing" : spaceHeld ? "grab" : "";
}

/** Screen-space handles of the selected shape, from its record. */
function handles(rec) {
  const [, kind, x, y, a, b, , ,] = rec;
  const zoom = camera.zoom;
  const sx = (x - camera.x) * zoom;
  const sy = (y - camera.y) * zoom;
  if (kind === POSTIT) return { resize: [sx + a * zoom, sy + b * zoom] };
  const half = (STICKER_SIZE / 2) * a * zoom;
  const cos = Math.cos(b);
  const sin = Math.sin(b);
  const at = (lx, ly) => [sx + lx * cos - ly * sin, sy + lx * sin + ly * cos];
  return { scale: at(half, half), rotate: at(0, -half - ROTATE_HANDLE_GAP_PX) };
}

function handleAt(sx, sy) {
  const rec = selected && !editing ? board.shapeRecord(selected) : undefined;
  if (!rec) return null;
  for (const [name, [hx, hy]] of Object.entries(handles(rec))) {
    if (Math.abs(sx - hx) <= HANDLE_PX && Math.abs(sy - hy) <= HANDLE_PX) return { name, rec };
  }
  return null;
}

canvas.addEventListener("pointerdown", (e) => {
  setTray(false);
  if (e.button === 1 || (e.button === 0 && spaceHeld)) {
    // Avoids middle-button autoscroll/paste.
    e.preventDefault();
    gesture = { kind: "pan", lastX: e.offsetX, lastY: e.offsetY };
  } else if (e.button === 0) {
    const wx = camera.toWorldX(e.offsetX);
    const wy = camera.toWorldY(e.offsetY);
    if (stamp) {
      const id = newId();
      board.addSticker(id, stamp, wx, wy);
      setStamp(null);
      select(id);
      edited();
      return;
    }
    const handle = handleAt(e.offsetX, e.offsetY);
    if (handle) {
      const [, , x, y, a, b] = handle.rec;
      if (handle.name === "resize") gesture = { kind: "resize", id: selected, x, y };
      else if (handle.name === "scale") gesture = { kind: "scale", id: selected, x, y, rotation: b };
      else gesture = { kind: "rotate", id: selected, x, y, scale: a };
    } else {
      const id = board.hitTest(wx, wy);
      if (editing && id === editing) return;
      select(id ?? null);
      if (!id) return;
      const [, , x, y] = board.shapeRecord(id);
      gesture = { kind: "drag", id, dx: wx - x, dy: wy - y };
    }
  } else {
    return;
  }
  canvas.setPointerCapture(e.pointerId);
  updateCursorStyle();
});

canvas.addEventListener("pointermove", (e) => {
  pointer.x = e.offsetX;
  pointer.y = e.offsetY;
  pointer.inside = true;
  if (gesture?.kind === "pan") {
    camera.pan(e.offsetX - gesture.lastX, e.offsetY - gesture.lastY);
    gesture.lastX = e.offsetX;
    gesture.lastY = e.offsetY;
    cameraChanged();
    return;
  }
  sendCursor(e.offsetX, e.offsetY);
  const wx = camera.toWorldX(e.offsetX);
  const wy = camera.toWorldY(e.offsetY);
  switch (gesture?.kind) {
    case "drag":
      board.moveShape(gesture.id, wx - gesture.dx, wy - gesture.dy);
      break;
    case "resize":
      board.resizePostit(gesture.id, Math.max(POSTIT_MIN, wx - gesture.x), Math.max(POSTIT_MIN, wy - gesture.y));
      break;
    case "scale": {
      const corner = (STICKER_SIZE / 2) * Math.SQRT2;
      const scale = Math.hypot(wx - gesture.x, wy - gesture.y) / corner;
      board.transformSticker(
        gesture.id,
        Math.min(STICKER_MAX_SCALE, Math.max(STICKER_MIN_SCALE, scale)),
        gesture.rotation,
      );
      break;
    }
    case "rotate": {
      // The handle sits above the center, i.e. at -90° when unrotated.
      let angle = Math.atan2(wy - gesture.y, wx - gesture.x) + Math.PI / 2;
      if (e.shiftKey) angle = Math.round(angle / ROTATE_STEP) * ROTATE_STEP;
      board.transformSticker(gesture.id, gesture.scale, angle);
      break;
    }
    default:
      return;
  }
  edited();
});

function endGesture() {
  gesture = null;
  updateCursorStyle();
  changed();
}
canvas.addEventListener("pointerup", endGesture);
canvas.addEventListener("pointercancel", endGesture);
canvas.addEventListener("pointerleave", () => (pointer.inside = false));
canvas.addEventListener("auxclick", (e) => e.preventDefault());

/** Creates a post-it centered on the screen point and starts editing it. */
function createPostit(sx, sy) {
  const id = newId();
  const wx = camera.toWorldX(sx);
  const wy = camera.toWorldY(sy);
  board.addPostit(id, wx - NEW_POSTIT_W / 2, wy - NEW_POSTIT_H / 2, NEW_POSTIT_W, NEW_POSTIT_H, "yellow");
  edited();
  startEditing(id);
}

canvas.addEventListener("dblclick", (e) => {
  if (spaceHeld || stamp) return;
  const id = board.hitTest(camera.toWorldX(e.offsetX), camera.toWorldY(e.offsetY));
  if (id === undefined) createPostit(e.offsetX, e.offsetY);
  else startEditing(id);
});

function onWheel(e) {
  // Without this, Ctrl+wheel/pinch zooms the whole page.
  e.preventDefault();
  const r = canvas.getBoundingClientRect();
  const sx = e.clientX - r.left;
  const sy = e.clientY - r.top;
  if (e.ctrlKey || e.metaKey) {
    camera.wheelZoom(sx, sy, e.deltaY, e.deltaMode);
  } else {
    const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? canvas.clientHeight : 1;
    let dx = e.deltaX * unit;
    let dy = e.deltaY * unit;
    // Shift + plain mouse wheel: horizontal scroll.
    if (e.shiftKey && dx === 0) [dx, dy] = [dy, 0];
    camera.pan(-dx, -dy);
  }
  pointer.x = sx;
  pointer.y = sy;
  pointer.inside = true;
  cameraChanged();
}
canvas.addEventListener("wheel", onWheel, { passive: false });

function typing(target) {
  return target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
}

/** Rotates the selected sticker by `steps` × 15°. */
function rotateSelected(steps) {
  const rec = selected && board.shapeRecord(selected);
  if (!rec || rec[1] !== STICKER) return;
  board.transformSticker(selected, rec[4], rec[5] + steps * ROTATE_STEP);
  edited();
}

addEventListener("keydown", (e) => {
  if (typing(e.target)) return;
  if (e.code === "Space") {
    e.preventDefault();
    if (!spaceHeld) {
      spaceHeld = true;
      updateCursorStyle();
    }
    return;
  }
  const mod = e.ctrlKey || e.metaKey;
  const plain = !mod && !e.altKey;
  if (e.code === "Digit0" && (e.shiftKey || mod) && !e.altKey) {
    e.preventDefault();
    camera.resetZoom(canvas.clientWidth, canvas.clientHeight);
    cameraChanged();
  } else if (e.code === "Digit1" && e.shiftKey && plain) {
    e.preventDefault();
    camera.zoomToFit(board, canvas.clientWidth, canvas.clientHeight);
    cameraChanged();
  } else if (e.code === "KeyN" && plain && !e.shiftKey && !e.repeat) {
    e.preventDefault();
    const [sx, sy] = pointer.inside ? [pointer.x, pointer.y] : [canvas.clientWidth / 2, canvas.clientHeight / 2];
    createPostit(sx, sy);
  } else if ((e.key === "Delete" || e.key === "Backspace") && plain && selected) {
    e.preventDefault();
    board.removeShape(selected);
    select(null);
    edited();
  } else if (e.key === "Enter" && plain && selected) {
    e.preventDefault();
    startEditing(selected);
  } else if ((e.key === "[" || e.key === "]") && !mod) {
    e.preventDefault();
    rotateSelected(e.key === "]" ? 1 : -1);
  } else if (e.key === "Escape") {
    if (stamp || !trayEl.hidden) {
      setStamp(null);
      setTray(false);
    } else {
      select(null);
    }
  }
});
addEventListener("keyup", (e) => {
  if (e.code !== "Space" || typing(e.target)) return;
  // Also keeps space from "clicking" a focused toolbar button.
  e.preventDefault();
  spaceHeld = false;
  updateCursorStyle();
});
addEventListener("blur", () => {
  spaceHeld = false;
  updateCursorStyle();
});

zoomEl.addEventListener("click", () => {
  zoomEl.blur();
  camera.resetZoom(canvas.clientWidth, canvas.clientHeight);
  cameraChanged();
});

// ---- drawing --------------------------------------------------------------------

function resize() {
  const dpr = devicePixelRatio || 1;
  canvas.width = canvas.clientWidth * dpr;
  canvas.height = canvas.clientHeight * dpr;
  changed();
}
addEventListener("resize", resize);
resize();

// Dot grid (FigJam style): world spacing that doubles when the dots would be
// too dense on screen.
const GRID_STEP = 20;
const GRID_MIN_PX = 12;
const GRID_DOT = 1.5;

function drawGrid(w, h, zoom, camX, camY) {
  let step = GRID_STEP;
  while (step * zoom < GRID_MIN_PX) step *= 2;
  const px = step * zoom;
  // First grid dot right/below the screen corner.
  const x0 = (Math.ceil(camX / step) * step - camX) * zoom;
  const y0 = (Math.ceil(camY / step) * step - camY) * zoom;
  const r = GRID_DOT / 2;
  ctx.fillStyle = "rgba(128, 128, 128, 0.35)";
  ctx.beginPath();
  for (let y = y0; y < h; y += px) {
    for (let x = x0; x < w; x += px) ctx.rect(x - r, y - r, GRID_DOT, GRID_DOT);
  }
  ctx.fill();
}

const EMOJI_FONT = '"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif';

/** Draws visible shapes in world coordinates (transform already set). */
function drawShapes(g, zoom, skipTextIndex) {
  ctx.lineWidth = 1 / zoom;
  ctx.strokeStyle = "rgba(0, 0, 0, 0.18)";
  for (let i = 0; i < g.length; i += RECORD) {
    const x = g[i + 2];
    const y = g[i + 3];
    if (g[i + 1] === POSTIT) {
      const w = g[i + 4];
      const h = g[i + 5];
      ctx.fillStyle = POSTIT_FILLS[g[i + 6]];
      ctx.fillRect(x, y, w, h);
      ctx.strokeRect(x, y, w, h);
      const version = g[i + 7];
      if (version === 0 || g[i] === skipTextIndex) continue;
      const { size, lines } = cachedLayout(g[i], version, w, h);
      if (size * zoom < MIN_READABLE_PX) continue;
      ctx.save();
      ctx.beginPath();
      ctx.rect(x, y, w, h);
      ctx.clip();
      ctx.fillStyle = TEXT_COLOR;
      ctx.font = `${size}px ${FONT_FAMILY}`;
      ctx.textBaseline = "top";
      const lh = size * LINE_HEIGHT;
      // Center each line box vertically on its glyphs, like the textarea.
      const top = y + TEXT_PAD + (lh - size) / 2;
      for (let k = 0; k < lines.length; k++) ctx.fillText(lines[k], x + TEXT_PAD, top + k * lh);
      ctx.restore();
    } else {
      const scale = g[i + 4];
      ctx.save();
      ctx.translate(x, y);
      ctx.rotate(g[i + 5]);
      ctx.font = `${STICKER_SIZE * scale * 0.85}px ${EMOJI_FONT}`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillStyle = TEXT_COLOR;
      ctx.fillText(STICKERS[g[i + 6]], 0, 0);
      ctx.restore();
    }
  }
}

/** Selection outline and handles, in screen pixels (transform = dpr only). */
function drawSelection(rec, zoom, camX, camY) {
  const [, kind, x, y, a, b] = rec;
  const sx = (x - camX) * zoom;
  const sy = (y - camY) * zoom;
  ctx.strokeStyle = SELECTION;
  ctx.fillStyle = "#fff";
  ctx.lineWidth = 2;
  const h = handles(rec);
  if (kind === POSTIT) {
    ctx.strokeRect(sx - 1, sy - 1, a * zoom + 2, b * zoom + 2);
  } else {
    const half = (STICKER_SIZE / 2) * a * zoom;
    ctx.save();
    ctx.translate(sx, sy);
    ctx.rotate(b);
    ctx.strokeRect(-half, -half, 2 * half, 2 * half);
    ctx.beginPath();
    ctx.moveTo(0, -half);
    ctx.lineTo(0, -half - ROTATE_HANDLE_GAP_PX);
    ctx.stroke();
    ctx.restore();
  }
  if (editing) return;
  ctx.lineWidth = 1.5;
  for (const [name, [hx, hy]] of Object.entries(h)) {
    ctx.beginPath();
    if (name === "rotate") ctx.arc(hx, hy, HANDLE_PX / 2 + 1, 0, 2 * Math.PI);
    else ctx.rect(hx - HANDLE_PX / 2, hy - HANDLE_PX / 2, HANDLE_PX, HANDLE_PX);
    ctx.fill();
    ctx.stroke();
  }
}

/** Floats the color toolbar above (or below) the selected post-it. */
function placeColorToolbar(rec, zoom, camX, camY, h) {
  if (!rec || rec[1] !== POSTIT || gesture) {
    colorsEl.hidden = true;
    return;
  }
  const [, , x, y, w, ph, color] = rec;
  colorsEl.hidden = false;
  for (const b of colorsEl.children) {
    b.setAttribute("aria-pressed", String(b.dataset.color === COLOR_NAMES[color]));
  }
  const cx = (x + w / 2 - camX) * zoom;
  const top = (y - camY) * zoom;
  const bottom = (y + ph - camY) * zoom;
  const tw = colorsEl.offsetWidth;
  const th = colorsEl.offsetHeight;
  // Above the post-it unless it would hide under the page toolbar.
  const ty = top - th - 12 > 48 || bottom + th + 12 > h ? top - th - 12 : bottom + 12;
  colorsEl.style.transform = `translate(${Math.round(cx - tw / 2)}px, ${Math.round(ty)}px)`;
}

function frame(now) {
  for (const [name, c] of cursors) {
    if (now - c.seen > 10_000) {
      cursors.delete(name);
      dirty = true;
    }
  }
  if (dirty) {
    dirty = false;
    const dpr = devicePixelRatio || 1;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    const zoom = camera.zoom;
    const camX = camera.x;
    const camY = camera.y;
    const sel = selected ? board.shapeRecord(selected) : undefined;
    if (selected && !sel) select(null);

    // Background and grid in screen pixels.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    drawGrid(w, h, zoom, camX, camY);

    // Shapes in world coordinates; Rust already culled what is off screen.
    const s = dpr * zoom;
    ctx.setTransform(s, 0, 0, s, -camX * s, -camY * s);
    drawShapes(board.visibleShapes(camera, w, h), zoom, editing && sel ? sel[0] : -1);

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (sel) drawSelection(sel, zoom, camX, camY);
    placeColorToolbar(sel, zoom, camX, camY, h);
    if (editing && sel) placeEditor(sel);

    // Other users' cursors: world position, fixed on-screen size.
    ctx.font = "12px system-ui";
    for (const [name, c] of cursors) {
      const x = (c.x - camX) * zoom;
      const y = (c.y - camY) * zoom;
      ctx.fillStyle = colorFor(name);
      ctx.beginPath();
      ctx.moveTo(x, y);
      ctx.lineTo(x + 12, y + 4);
      ctx.lineTo(x + 4, y + 12);
      ctx.fill();
      ctx.fillText(name, x + 14, y + 20);
    }
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

connect();

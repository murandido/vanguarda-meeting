// Cartazes — retângulos móveis sincronizados via Loro, num quadro infinito.
// O estado (documento e câmera) mora no WASM; aqui só tem eventos, rede e
// desenho. Coordenadas: "tela" = pixels CSS do canvas; "mundo" = coordenadas
// do quadro, iguais para todos. A câmera (pan/zoom) é local de cada um.
import init, { Board, Camera } from "/pkg/canvas.js";

await init();

const canvas = document.getElementById("board");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const zoomEl = document.getElementById("zoom");
const boardId = canvas.dataset.board;

// Login da sessão; o servidor ignora o nome que o cliente mandar.
const user = canvas.dataset.user;

const board = new Board();
const camera = new Camera();
const cursors = new Map(); // user -> { x, y, seen } (mundo)
let ws;
let dirty = true;

function colorFor(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return `hsl(${h} 70% 45%)`;
}

/** Algo visível mudou: redesenha no próximo frame. */
function changed() {
  dirty = true;
}

/** A câmera mudou: redesenha, atualiza o zoom na toolbar e o cursor remoto. */
function cameraChanged() {
  changed();
  zoomEl.textContent = `${Math.round(camera.zoom * 100)}%`;
  // O ponteiro parado agora aponta para outro ponto do mundo.
  if (pointer.inside) sendCursor(pointer.x, pointer.y);
}

// ---- rede ----------------------------------------------------------------

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
/** Envia o cursor em coordenadas do MUNDO, a partir de um ponto da tela. */
function sendCursor(sx, sy) {
  const now = performance.now();
  if (ws?.readyState !== WebSocket.OPEN || now - lastCursorSent < 33) return;
  lastCursorSent = now;
  ws.send(JSON.stringify({ type: "cursor", user, x: camera.toWorldX(sx), y: camera.toWorldY(sy) }));
}

// ---- interação -----------------------------------------------------------
//
// Pan: espaço + arraste, botão do meio, ou rolagem com dois dedos (wheel sem
// Ctrl). Zoom: Ctrl/Cmd + roda ou pinça (wheel com ctrlKey), em torno do
// ponteiro. Teclado: Shift+0 / Ctrl+0 = 100%, Shift+1 = enquadrar tudo,
// Ctrl/Cmd+Z = desfazer, Ctrl/Cmd+Shift+Z ou Ctrl+Y = refazer.

const pointer = { x: 0, y: 0, inside: false };
let spaceHeld = false;
let gesture = null; // { kind: "pan", lastX, lastY } | { kind: "drag", id, dx, dy }

function updateCursorStyle() {
  canvas.style.cursor = gesture?.kind === "pan" ? "grabbing" : spaceHeld ? "grab" : "";
}

canvas.addEventListener("pointerdown", (e) => {
  if (e.button === 1 || (e.button === 0 && spaceHeld)) {
    // Evita o autoscroll/colar do botão do meio.
    e.preventDefault();
    gesture = { kind: "pan", lastX: e.offsetX, lastY: e.offsetY };
  } else if (e.button === 0) {
    const wx = camera.toWorldX(e.offsetX);
    const wy = camera.toWorldY(e.offsetY);
    const hit = board.hitTest(wx, wy);
    if (!hit) return;
    gesture = { kind: "drag", id: hit.id, dx: wx - hit.x, dy: wy - hit.y };
    hit.free();
    // The whole drag undoes as one step.
    board.beginGroup();
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
  if (gesture?.kind === "drag") {
    board.moveShape(gesture.id, camera.toWorldX(e.offsetX) - gesture.dx, camera.toWorldY(e.offsetY) - gesture.dy);
    flush();
    changed();
  }
});

function endGesture() {
  if (gesture?.kind === "drag") board.endGroup();
  gesture = null;
  updateCursorStyle();
}
canvas.addEventListener("pointerup", endGesture);
canvas.addEventListener("pointercancel", endGesture);
canvas.addEventListener("pointerleave", () => (pointer.inside = false));
canvas.addEventListener("auxclick", (e) => e.preventDefault());

canvas.addEventListener("dblclick", (e) => {
  if (spaceHeld) return;
  const wx = camera.toWorldX(e.offsetX);
  const wy = camera.toWorldY(e.offsetY);
  board.addRect(crypto.randomUUID(), wx - 60, wy - 40, 120, 80);
  flush();
  changed();
});

canvas.addEventListener(
  "wheel",
  (e) => {
    // Sem isso, Ctrl+roda/pinça dá zoom na página inteira.
    e.preventDefault();
    if (e.ctrlKey || e.metaKey) {
      camera.wheelZoom(e.offsetX, e.offsetY, e.deltaY, e.deltaMode);
    } else {
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? canvas.clientHeight : 1;
      let dx = e.deltaX * unit;
      let dy = e.deltaY * unit;
      // Shift + roda de mouse comum: rolagem horizontal.
      if (e.shiftKey && dx === 0) [dx, dy] = [dy, 0];
      camera.pan(-dx, -dy);
    }
    pointer.x = e.offsetX;
    pointer.y = e.offsetY;
    pointer.inside = true;
    cameraChanged();
  },
  { passive: false },
);

function typing(target) {
  return target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
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
  // Layout-aware letter (e.key), like native Ctrl+Z.
  const letter = e.key.toLowerCase();
  if (mod && !e.altKey && (letter === "z" || (letter === "y" && !e.shiftKey))) {
    // Undo/redo only this user's own edits; text fields keep native undo
    // (filtered by typing() above). Ignored mid-drag.
    e.preventDefault();
    if (gesture?.kind === "drag") return;
    const redo = letter === "y" || e.shiftKey;
    if (redo ? board.redo() : board.undo()) {
      flush();
      changed();
    }
  } else if (e.code === "Digit0" && (e.shiftKey || mod) && !e.altKey) {
    e.preventDefault();
    camera.resetZoom(canvas.clientWidth, canvas.clientHeight);
    cameraChanged();
  } else if (e.code === "Digit1" && e.shiftKey && !mod && !e.altKey) {
    e.preventDefault();
    camera.zoomToFit(board, canvas.clientWidth, canvas.clientHeight);
    cameraChanged();
  }
});
addEventListener("keyup", (e) => {
  if (e.code !== "Space" || typing(e.target)) return;
  // Também impede que o espaço "clique" o botão de zoom focado.
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

// ---- desenho -------------------------------------------------------------

function resize() {
  const dpr = devicePixelRatio || 1;
  canvas.width = canvas.clientWidth * dpr;
  canvas.height = canvas.clientHeight * dpr;
  changed();
}
addEventListener("resize", resize);
resize();

// Grade de pontos (estilo FigJam): espaçamento no mundo que dobra quando os
// pontos ficariam apertados demais na tela.
const GRID_STEP = 20;
const GRID_MIN_PX = 12;
const GRID_DOT = 1.5;

function drawGrid(w, h, zoom, camX, camY) {
  let step = GRID_STEP;
  while (step * zoom < GRID_MIN_PX) step *= 2;
  const px = step * zoom;
  // Primeiro ponto da grade à direita/abaixo do canto da tela.
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

    // Fundo e grade em pixels de tela.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    drawGrid(w, h, zoom, camX, camY);

    // Formas em coordenadas do mundo; o Rust já recortou o que está fora da tela.
    const s = dpr * zoom;
    ctx.setTransform(s, 0, 0, s, -camX * s, -camY * s);
    ctx.fillStyle = "#ffe066";
    ctx.strokeStyle = "#c9a400";
    ctx.lineWidth = 1 / zoom;
    const g = board.visibleGeometry(camera, w, h);
    for (let i = 0; i < g.length; i += 4) {
      ctx.fillRect(g[i], g[i + 1], g[i + 2], g[i + 3]);
      ctx.strokeRect(g[i], g[i + 1], g[i + 2], g[i + 3]);
    }

    // Cursores dos outros: posição no mundo, desenho em tamanho fixo na tela.
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
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

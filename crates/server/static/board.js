// Cartazes — spike da Fase 0: retângulos móveis sincronizados via Loro.
// O estado mora no WASM; aqui só tem eventos, rede e desenho.
import init, { Board } from "/pkg/canvas.js";

await init();

const canvas = document.getElementById("board");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const boardId = canvas.dataset.board;

const user = (() => {
  try {
    let name = localStorage.getItem("vanguarda-user");
    if (!name) {
      name = "anon-" + Math.random().toString(36).slice(2, 6);
      localStorage.setItem("vanguarda-user", name);
    }
    return name;
  } catch {
    return "anon-" + Math.random().toString(36).slice(2, 6);
  }
})();

const board = new Board();
const cursors = new Map(); // user -> { x, y, seen }
let ws;
let dirty = true;

function colorFor(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return `hsl(${h} 70% 45%)`;
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
    dirty = true;
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
function sendCursor(x, y) {
  const now = performance.now();
  if (ws?.readyState !== WebSocket.OPEN || now - lastCursorSent < 33) return;
  lastCursorSent = now;
  ws.send(JSON.stringify({ type: "cursor", user, x, y }));
}

// ---- interação -----------------------------------------------------------

function hit(x, y) {
  const g = board.rectGeometry();
  const ids = board.rectIds();
  for (let i = ids.length - 1; i >= 0; i--) {
    const [rx, ry, rw, rh] = g.subarray(i * 4, i * 4 + 4);
    if (x >= rx && x <= rx + rw && y >= ry && y <= ry + rh) return { id: ids[i], dx: x - rx, dy: y - ry };
  }
  return null;
}

let drag = null;
canvas.addEventListener("pointerdown", (e) => {
  drag = hit(e.offsetX, e.offsetY);
  if (drag) canvas.setPointerCapture(e.pointerId);
});
canvas.addEventListener("pointermove", (e) => {
  sendCursor(e.offsetX, e.offsetY);
  if (!drag) return;
  board.moveShape(drag.id, e.offsetX - drag.dx, e.offsetY - drag.dy);
  flush();
  dirty = true;
});
canvas.addEventListener("pointerup", () => (drag = null));
canvas.addEventListener("dblclick", (e) => {
  board.addRect(crypto.randomUUID(), e.offsetX - 60, e.offsetY - 40, 120, 80);
  flush();
  dirty = true;
});

// ---- desenho -------------------------------------------------------------

function resize() {
  const dpr = devicePixelRatio || 1;
  canvas.width = canvas.clientWidth * dpr;
  canvas.height = canvas.clientHeight * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  dirty = true;
}
addEventListener("resize", resize);
resize();

function frame(now) {
  for (const [name, c] of cursors) {
    if (now - c.seen > 10_000) {
      cursors.delete(name);
      dirty = true;
    }
  }
  if (dirty) {
    dirty = false;
    ctx.clearRect(0, 0, canvas.clientWidth, canvas.clientHeight);
    const g = board.rectGeometry();
    for (let i = 0; i < g.length; i += 4) {
      ctx.fillStyle = "#ffe066";
      ctx.strokeStyle = "#c9a400";
      ctx.fillRect(g[i], g[i + 1], g[i + 2], g[i + 3]);
      ctx.strokeRect(g[i], g[i + 1], g[i + 2], g[i + 3]);
    }
    for (const [name, c] of cursors) {
      ctx.fillStyle = colorFor(name);
      ctx.beginPath();
      ctx.moveTo(c.x, c.y);
      ctx.lineTo(c.x + 12, c.y + 4);
      ctx.lineTo(c.x + 4, c.y + 12);
      ctx.fill();
      ctx.font = "12px system-ui";
      ctx.fillText(name, c.x + 14, c.y + 20);
    }
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

connect();

// Ritmo: atualização ao vivo por SSE e avisos de erro das ações HTMX.
//
// O servidor manda o evento "ritmo" a cada mudança; aqui ele vira o evento
// HTMX "ritmo-changed", que faz o #quadro buscar o parcial de novo. Ao
// reconectar (o EventSource tenta sozinho), recarrega também, para não perder
// o que mudou enquanto a conexão estava caída.
//
// Also: project colors (the CSP forbids inline styles, so `data-cor` is
// applied through the CSSOM), and the celebrations (KB-08).
(() => {
  const aviso = document.getElementById("aviso");

  // ---------- Project colors ----------
  const paint = (root) => {
    for (const el of root.querySelectorAll("[data-cor]")) {
      el.style.setProperty("--cor", el.dataset.cor);
    }
  };
  paint(document);
  htmx.onLoad(paint);

  document.body.addEventListener("htmx:responseError", (e) => {
    if (!aviso) return;
    aviso.textContent = e.detail.xhr.responseText || "Algo deu errado. Tente de novo.";
    aviso.hidden = false;
  });
  document.body.addEventListener("htmx:sendError", () => {
    if (!aviso) return;
    aviso.textContent = "Sem conexão com o servidor.";
    aviso.hidden = false;
  });
  document.body.addEventListener("htmx:afterRequest", (e) => {
    if (aviso && e.detail.successful) aviso.hidden = true;
    // After adding a task, clear the form for the next one.
    const form = e.detail.elt;
    if (e.detail.successful && form.id === "nova-tarefa") {
      form.reset();
      form.titulo.focus();
    }
  });

  // ---------- Celebrations ----------
  const SOUND_KEY = "ritmo-som";
  const storage = {
    get(key) {
      try {
        return localStorage.getItem(key);
      } catch {
        return null;
      }
    },
    set(key, value) {
      try {
        localStorage.setItem(key, value);
      } catch {
        // Private mode or blocked storage: the toggle just won't stick.
      }
    },
  };

  const soundToggle = document.getElementById("som");
  if (soundToggle) {
    soundToggle.checked = storage.get(SOUND_KEY) === "1";
    soundToggle.addEventListener("change", () => {
      storage.set(SOUND_KEY, soundToggle.checked ? "1" : "0");
    });
  }

  const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // A short rising arpeggio, synthesized: no audio file to download.
  const chime = () => {
    if (storage.get(SOUND_KEY) !== "1") return;
    try {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (!Ctx) return;
      const ctx = new Ctx();
      [523.25, 659.25, 783.99].forEach((freq, i) => {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        const start = ctx.currentTime + i * 0.09;
        osc.frequency.value = freq;
        gain.gain.setValueAtTime(0.0001, start);
        gain.gain.exponentialRampToValueAtTime(0.15, start + 0.02);
        gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.3);
        osc.connect(gain).connect(ctx.destination);
        osc.start(start);
        osc.stop(start + 0.32);
      });
      setTimeout(() => ctx.close(), 1000);
    } catch {
      // Audio is a nice-to-have.
    }
  };

  const COLORS = ["#4c6ef5", "#2b8a3e", "#f59f00", "#e64980", "#15aabf", "#fab005"];

  const confetti = () => {
    document.getElementById("confete")?.remove();
    const canvas = document.createElement("canvas");
    canvas.id = "confete";
    canvas.className = "confete";
    canvas.setAttribute("aria-hidden", "true");
    const ratio = window.devicePixelRatio || 1;
    const w = window.innerWidth;
    const h = window.innerHeight;
    canvas.width = w * ratio;
    canvas.height = h * ratio;
    document.body.append(canvas);
    const g = canvas.getContext("2d");
    if (!g) {
      canvas.remove();
      return;
    }
    g.scale(ratio, ratio);

    const pieces = Array.from({ length: 140 }, () => ({
      x: w / 2 + (Math.random() - 0.5) * w * 0.3,
      y: h * 0.35,
      vx: (Math.random() - 0.5) * 12,
      vy: -Math.random() * 11 - 4,
      size: 5 + Math.random() * 6,
      angle: Math.random() * Math.PI,
      spin: (Math.random() - 0.5) * 0.3,
      color: COLORS[Math.floor(Math.random() * COLORS.length)],
    }));
    const DURATION = 1800;
    const begin = performance.now();
    const frame = (now) => {
      const t = now - begin;
      g.clearRect(0, 0, w, h);
      g.globalAlpha = Math.max(0, 1 - t / DURATION);
      for (const p of pieces) {
        p.vy += 0.35;
        p.vx *= 0.99;
        p.x += p.vx;
        p.y += p.vy;
        p.angle += p.spin;
        g.save();
        g.translate(p.x, p.y);
        g.rotate(p.angle);
        g.fillStyle = p.color;
        g.fillRect(-p.size / 2, -p.size / 4, p.size, p.size / 2);
        g.restore();
      }
      if (t < DURATION) {
        requestAnimationFrame(frame);
      } else {
        canvas.remove();
      }
    };
    requestAnimationFrame(frame);
  };

  const toast = document.getElementById("comemoracao");
  let toastTimer;
  const celebrate = (message) => {
    if (toast) {
      toast.textContent = message;
      toast.hidden = false;
      clearTimeout(toastTimer);
      toastTimer = setTimeout(() => {
        toast.hidden = true;
      }, 2500);
    }
    if (!reducedMotion()) confetti();
    chime();
  };

  document.body.addEventListener("ritmo-celebrate", () => celebrate("Feito! 🎉"));
  if (location.hash === "#celebrar") {
    history.replaceState(null, "", location.pathname + location.search);
    celebrate("Check-in salvo! 🔥");
  }

  // ---------- Live updates ----------
  if (!document.getElementById("quadro")) return;
  const recarregar = () => htmx.trigger(document.body, "ritmo-changed");
  const fonte = new EventSource("/ritmo/events");
  let conectou = false;
  fonte.addEventListener("open", () => {
    if (conectou) recarregar();
    conectou = true;
  });
  fonte.addEventListener("ritmo", recarregar);
})();

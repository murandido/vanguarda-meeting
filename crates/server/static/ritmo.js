// Ritmo: atualização ao vivo por SSE e avisos de erro das ações HTMX.
//
// O servidor manda o evento "ritmo" a cada mudança; aqui ele vira o evento
// HTMX "ritmo-changed", que faz o #quadro buscar o parcial de novo. Ao
// reconectar (o EventSource tenta sozinho), recarrega também, para não perder
// o que mudou enquanto a conexão estava caída.
(() => {
  const aviso = document.getElementById("aviso");

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
  });

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

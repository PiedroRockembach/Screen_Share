// Painel do Sala de Tela (só neste PC). Conversa com o programa via /api.
"use strict";

const $ = (id) => document.getElementById(id);

// A chave vem no endereço que o programa abriu; sem ela a API recusa.
const chave = new URLSearchParams(location.search).get("k") || sessionStorage.getItem("chave") || "";
sessionStorage.setItem("chave", chave);
history.replaceState(null, "", "/");

async function api(caminho, corpo) {
  const r = await fetch("/api/" + caminho, {
    method: corpo === undefined ? "GET" : "POST",
    headers: { "X-Chave": chave, "Content-Type": "application/json" },
    body: corpo === undefined ? undefined : JSON.stringify(corpo),
  });
  const dados = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(dados.erro || `erro ${r.status}`);
  return dados;
}

async function acao(caminho, corpo = {}) {
  try {
    await api(caminho, corpo);
  } catch (e) {
    $("erro").textContent = e.message;
    $("erro").hidden = false;
  }
  atualizar();
}

let primeira = true;

function mostrar(e) {
  $("ocupado").hidden = !e.ocupado;
  $("ocupado").textContent = e.ocupado ? e.ocupado + "…" : "";

  $("acao").hidden = !e.textoAcao;
  $("acao-texto").textContent = e.textoAcao;
  $("acao-link").hidden = !e.urlAcao;
  $("acao-link").href = e.urlAcao || "#";

  $("erro").hidden = !e.erro;
  $("erro").textContent = e.erro;

  $("sala-fechada").hidden = e.salaAtiva;
  $("sala-aberta").hidden = !e.salaAtiva;
  $("btn-criar").disabled = !!e.ocupado;
  if ($("link").value !== e.linkSala) $("link").value = e.linkSala;
  $("btn-abrir").dataset.url = e.linkLocal;

  if (primeira) {
    $("nome").value = e.nome;
    $("com-som").checked = e.comSom;
    $("link-sala").value = e.linkUltimo;
    primeira = false;
  }
  $("btn-transmitir").textContent = e.transmitindo ? "■ Parar de transmitir" : "▶ Transmitir";
  $("btn-transmitir").disabled = !!e.ocupado && !e.transmitindo;
  for (const id of ["link-sala", "nome", "com-som"]) $(id).disabled = e.transmitindo;
  $("status-trans").textContent = e.statusTrans || "";

  const hist = $("historico");
  const noFim = hist.scrollTop + hist.clientHeight >= hist.scrollHeight - 4;
  hist.textContent = (e.historico || []).join("\n");
  if (noFim) hist.scrollTop = hist.scrollHeight;
}

async function atualizar() {
  try {
    mostrar(await api("estado"));
  } catch {
    $("ocupado").hidden = false;
    $("ocupado").textContent = "o Sala de Tela está fechado";
  }
}

$("btn-criar").onclick = () => acao("sala/criar");
$("btn-fechar").onclick = () => confirm("Fechar a sala? Quem estiver assistindo cai.") && acao("sala/fechar");
$("btn-novo-link").onclick = () =>
  confirm("Criar um link novo? O link atual para de funcionar e você precisa mandar o novo para todos.") &&
  acao("sala/novo-link");
$("btn-copiar").onclick = async () => {
  await navigator.clipboard.writeText($("link").value).catch(() => $("link").select());
  $("btn-copiar").textContent = "Copiado!";
  setTimeout(() => ($("btn-copiar").textContent = "Copiar"), 1500);
};
$("btn-abrir").onclick = () => window.open($("btn-abrir").dataset.url, "_blank", "noopener");
$("btn-transmitir").onclick = () => {
  if ($("btn-transmitir").textContent.startsWith("■")) return acao("transmitir/parar");
  acao("transmitir", { link: $("link-sala").value, nome: $("nome").value, comSom: $("com-som").checked });
};
$("btn-encerrar").onclick = () =>
  confirm("Encerrar o Sala de Tela? A sala fecha e a transmissão para.") && acao("encerrar");

atualizar();
setInterval(atualizar, 1000);

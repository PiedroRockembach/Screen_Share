// Sala de Tela: lista quem está ao vivo e toca a transmissão via WHEP (WebRTC).
"use strict";

const WHEP_BASE = `${location.protocol}//${location.hostname}:8889`;
const INTERVALO_LISTA_MS = 3000;

const el = {
  status: document.getElementById("status-servidor"),
  lista: document.getElementById("lista"),
  vazio: document.getElementById("vazio"),
  video: document.getElementById("video"),
  aviso: document.getElementById("aviso"),
  assistindo: document.getElementById("assistindo"),
  btnSom: document.getElementById("btn-som"),
  volume: document.getElementById("volume"),
  btnCheia: document.getElementById("btn-cheia"),
  btnParar: document.getElementById("btn-parar"),
};

let atual = null;        // nome da transmissão escolhida
let pc = null;           // RTCPeerConnection ativa
let sessaoUrl = null;    // URL da sessão WHEP (para encerrar com DELETE)
let tentativa = null;    // timer de reconexão
let aoVivo = [];

// ---------- lista de transmissões ----------

async function atualizarLista() {
  let ok = false;
  try {
    const r = await fetch("/api/ao-vivo", { cache: "no-store" });
    if (!r.ok) throw new Error(r.status);
    aoVivo = await r.json();
    el.status.textContent = "conectado";
    el.status.className = "pilula ok";
    ok = true;
  } catch {
    aoVivo = [];
    el.status.textContent = "servidor fora do ar";
    el.status.className = "pilula erro";
  }
  desenharLista();

  // A transmissão que eu assistia acabou.
  if (ok && atual && !aoVivo.some((t) => t.nome === atual)) {
    parar();
    el.aviso.textContent = "A transmissão terminou.";
  }

  // Se ninguém foi escolhido e só há uma transmissão, já começa a assistir.
  if (!atual && aoVivo.length === 1) assistir(aoVivo[0].nome);
}

function desenharLista() {
  el.lista.replaceChildren();
  el.vazio.hidden = aoVivo.length > 0;
  for (const t of aoVivo) {
    const li = document.createElement("li");
    const b = document.createElement("button");
    b.className = t.nome === atual ? "ativo" : "";
    const nome = document.createElement("div");
    nome.className = "nome";
    nome.textContent = t.nome;
    const meta = document.createElement("div");
    meta.className = "meta";
    const temAudio = t.faixas.some((f) => /opus/i.test(f));
    meta.textContent = `${t.espectadores} assistindo · ${temAudio ? "com som" : "sem som"}`;
    b.append(nome, meta);
    b.onclick = () => assistir(t.nome);
    li.append(b);
    el.lista.append(li);
  }
}

// ---------- WHEP ----------

function esperarIce(conexao, limiteMs) {
  if (conexao.iceGatheringState === "complete") return Promise.resolve();
  return new Promise((ok) => {
    const fim = () => { clearTimeout(t); ok(); };
    const t = setTimeout(fim, limiteMs);
    conexao.addEventListener("icegatheringstatechange", () => {
      if (conexao.iceGatheringState === "complete") fim();
    });
  });
}

async function conectar(nome) {
  // Dentro da VPN não é preciso STUN/TURN: os IPs do Tailscale já se alcançam.
  const conexao = new RTCPeerConnection({ iceServers: [] });
  pc = conexao;
  conexao.addTransceiver("video", { direction: "recvonly" });
  conexao.addTransceiver("audio", { direction: "recvonly" });

  const midia = new MediaStream();
  conexao.ontrack = (ev) => {
    if ("jitterBufferTarget" in ev.receiver) ev.receiver.jitterBufferTarget = 0;
    midia.addTrack(ev.track);
    el.video.srcObject = midia;
  };
  conexao.onconnectionstatechange = () => {
    if (conexao !== pc) return;
    const estado = conexao.connectionState;
    if (estado === "connected") {
      el.aviso.textContent = "";
    } else if (estado === "failed" || estado === "disconnected") {
      agendarReconexao();
    }
  };

  const oferta = await conexao.createOffer();
  await conexao.setLocalDescription(oferta);
  await esperarIce(conexao, 1500);

  const url = `${WHEP_BASE}/${encodeURIComponent(nome)}/whep`;
  const r = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/sdp" },
    body: conexao.localDescription.sdp,
  });
  if (r.status !== 201) throw new Error(`WHEP respondeu ${r.status}`);
  const local = r.headers.get("Location");
  sessaoUrl = local ? new URL(local, url).href : null;
  await conexao.setRemoteDescription({ type: "answer", sdp: await r.text() });
}

function encerrar() {
  clearTimeout(tentativa);
  if (sessaoUrl) fetch(sessaoUrl, { method: "DELETE" }).catch(() => {});
  sessaoUrl = null;
  if (pc) pc.close();
  pc = null;
  el.video.srcObject = null;
}

function agendarReconexao() {
  clearTimeout(tentativa);
  el.aviso.textContent = "Reconectando…";
  tentativa = setTimeout(() => { if (atual) assistir(atual); }, 2000);
}

async function assistir(nome) {
  encerrar();
  atual = nome;
  desenharLista();
  el.aviso.textContent = "Conectando…";
  el.assistindo.textContent = `Assistindo: ${nome}`;
  for (const b of [el.btnSom, el.btnCheia, el.btnParar]) b.disabled = false;
  try {
    await conectar(nome);
  } catch (e) {
    console.warn(e);
    if (atual === nome) agendarReconexao();
  }
}

function parar() {
  encerrar();
  atual = null;
  el.aviso.textContent = "Escolha uma transmissão na lista.";
  el.assistindo.textContent = "";
  for (const b of [el.btnSom, el.btnCheia, el.btnParar]) b.disabled = true;
  desenharLista();
}

// ---------- controles ----------

function atualizarBotaoSom() {
  const mudo = el.video.muted;
  el.btnSom.textContent = mudo ? "🔇 Ativar som" : "🔊 Som ligado";
  el.btnSom.classList.toggle("ligado", !mudo);
}

el.btnSom.onclick = () => {
  // O navegador só deixa tocar som depois de um clique do usuário.
  el.video.muted = !el.video.muted;
  el.video.play().catch(() => {});
  atualizarBotaoSom();
};
el.volume.oninput = () => { el.video.volume = Number(el.volume.value); };
el.btnCheia.onclick = () => document.getElementById("moldura").requestFullscreen?.();
el.video.ondblclick = () => el.btnCheia.click();
el.btnParar.onclick = parar;

window.addEventListener("pagehide", encerrar);

atualizarBotaoSom();
atualizarLista();
setInterval(atualizarLista, INTERVALO_LISTA_MS);

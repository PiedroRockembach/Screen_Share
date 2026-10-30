// Sala de Tela: lista quem está ao vivo, toca via WHEP e transmite via WHIP.
// A página fica em https://<pc>.ts.net/s/<código>/ e tudo é relativo a ela.
"use strict";

const INTERVALO_LISTA_MS = 3000;
// STUN gratuito: descobre o IP público para o vídeo ir direto entre os PCs.
const ICE = [{ urls: ["stun:stun.l.google.com:19302", "stun:stun.cloudflare.com:3478"] }];

const $ = (id) => document.getElementById(id);
const el = {
  status: $("status-servidor"),
  lista: $("lista"),
  vazio: $("vazio"),
  video: $("video"),
  aviso: $("aviso"),
  assistindo: $("assistindo"),
  btnSom: $("btn-som"),
  volume: $("volume"),
  btnCheia: $("btn-cheia"),
  btnParar: $("btn-parar"),
  nome: $("nome"),
  btnTransmitir: $("btn-transmitir"),
  statusTrans: $("status-trans"),
};

const endereco = (caminho) => new URL(caminho, location.href).href;

let atual = null;        // nome da transmissão que estou assistindo
let pc = null;           // RTCPeerConnection de quem assiste
let sessaoUrl = null;    // sessão WHEP (para encerrar com DELETE)
let tentativa = null;    // timer de reconexão
let aoVivo = [];

// ---------- comum ----------

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

// Faz a oferta, manda para o servidor (WHIP/WHEP) e aplica a resposta.
async function negociar(conexao, url) {
  await conexao.setLocalDescription(await conexao.createOffer());
  await esperarIce(conexao, 2000);
  const r = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/sdp" },
    body: conexao.localDescription.sdp,
  });
  if (r.status !== 201) throw new Error(`o servidor respondeu ${r.status}`);
  const local = r.headers.get("Location");
  await conexao.setRemoteDescription({ type: "answer", sdp: await r.text() });
  return local ? new URL(local, url).href : null;
}

// ---------- lista de transmissões ----------

async function atualizarLista() {
  let ok = false;
  try {
    const r = await fetch(endereco("api/ao-vivo"), { cache: "no-store" });
    if (!r.ok) throw new Error(r.status);
    aoVivo = await r.json();
    el.status.textContent = "conectado";
    el.status.className = "pilula ok";
    ok = true;
  } catch {
    aoVivo = [];
    el.status.textContent = "sala fechada ou fora do ar";
    el.status.className = "pilula erro";
  }
  desenharLista();

  if (ok && atual && !aoVivo.some((t) => t.nome === atual)) {
    parar();
    el.aviso.textContent = "A transmissão terminou.";
  }
  // Ninguém escolhido e só uma transmissão (que não é a minha): já começa a assistir.
  const outras = aoVivo.filter((t) => t.nome !== transmitindoComo);
  if (!atual && outras.length === 1) assistir(outras[0].nome);
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
    nome.textContent = t.nome + (t.nome === transmitindoComo ? " (você)" : "");
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

// ---------- assistir (WHEP) ----------

async function conectar(nome) {
  const conexao = new RTCPeerConnection({ iceServers: ICE });
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
    if (estado === "connected") el.aviso.textContent = "";
    else if (estado === "failed" || estado === "disconnected") agendarReconexao();
  };
  sessaoUrl = await negociar(conexao, endereco(`${encodeURIComponent(nome)}/whep`));
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
  el.aviso.textContent = "Reconectando… (se nunca conectar, a sua rede pode estar bloqueando a conexão direta)";
  tentativa = setTimeout(() => { if (atual) assistir(atual); }, 3000);
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

// ---------- transmitir pelo navegador (WHIP, só vídeo) ----------
// Sem som de propósito: o navegador capturaria a voz do Discord junto (eco).
// Para transmitir com som, use o app Sala de Tela.

let transmissao = null;   // { pc, midia, sessao }
let transmitindoComo = null;

function nomeSeguro(s) {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32);
}

async function transmitirNavegador() {
  const nome = nomeSeguro(el.nome.value);
  if (!nome) { el.statusTrans.textContent = "Escreva um nome."; return; }
  try { localStorage.setItem("sala-nome", nome); } catch {}
  el.nome.value = nome;

  let midia;
  try {
    midia = await navigator.mediaDevices.getDisplayMedia({
      video: { width: { ideal: 1920 }, height: { ideal: 1080 }, frameRate: { ideal: 60, max: 60 } },
      audio: false,
    });
  } catch {
    el.statusTrans.textContent = "Compartilhamento cancelado.";
    return;
  }
  const [faixa] = midia.getVideoTracks();
  faixa.contentHint = "motion"; // prioriza 60 fps em vez de nitidez de texto parado

  const conexao = new RTCPeerConnection({ iceServers: ICE });
  const tr = conexao.addTransceiver(faixa, {
    direction: "sendonly",
    streams: [midia],
    sendEncodings: [{ maxBitrate: 8_000_000, maxFramerate: 60 }],
  });
  const codecs = RTCRtpSender.getCapabilities?.("video")?.codecs;
  if (codecs && tr.setCodecPreferences) {
    const h264 = codecs.filter((c) => /h264/i.test(c.mimeType));
    if (h264.length) tr.setCodecPreferences([...h264, ...codecs.filter((c) => !/h264/i.test(c.mimeType))]);
  }
  try {
    const p = tr.sender.getParameters();
    p.degradationPreference = "maintain-framerate";
    await tr.sender.setParameters(p);
  } catch {}

  try {
    const sessao = await negociar(conexao, endereco(`${encodeURIComponent(nome)}/whip`));
    transmissao = { pc: conexao, midia, sessao };
    transmitindoComo = nome;
  } catch (e) {
    midia.getTracks().forEach((t) => t.stop());
    conexao.close();
    el.statusTrans.textContent = `Não deu para transmitir: ${e.message}`;
    return;
  }
  faixa.onended = pararNavegador; // clicou em "parar de compartilhar" do navegador
  el.btnTransmitir.textContent = "■ Parar de transmitir";
  el.statusTrans.textContent = `AO VIVO como "${nome}" (sem som).`;
}

function pararNavegador() {
  if (!transmissao) return;
  if (transmissao.sessao) fetch(transmissao.sessao, { method: "DELETE" }).catch(() => {});
  transmissao.midia.getTracks().forEach((t) => t.stop());
  transmissao.pc.close();
  transmissao = null;
  transmitindoComo = null;
  el.btnTransmitir.textContent = "▶ Transmitir minha tela";
  el.statusTrans.textContent = "Transmissão encerrada.";
}

// ---------- controles ----------

function atualizarBotaoSom() {
  const mudo = el.video.muted;
  el.btnSom.textContent = mudo ? "🔇 Ativar som" : "🔊 Som ligado";
  el.btnSom.classList.toggle("ligado", !mudo);
}

el.btnSom.onclick = () => {
  // O navegador só deixa tocar som depois de um clique.
  el.video.muted = !el.video.muted;
  el.video.play().catch(() => {});
  atualizarBotaoSom();
};
el.volume.oninput = () => { el.video.volume = Number(el.volume.value); };
el.btnCheia.onclick = () => $("moldura").requestFullscreen?.();
el.video.ondblclick = () => el.btnCheia.click();
el.btnParar.onclick = parar;
el.btnTransmitir.onclick = () => (transmissao ? pararNavegador() : transmitirNavegador());
if (!navigator.mediaDevices?.getDisplayMedia) {
  el.btnTransmitir.disabled = true;
  el.statusTrans.textContent = "Este navegador não transmite tela; use o app Sala de Tela.";
}
try { el.nome.value = localStorage.getItem("sala-nome") || ""; } catch {}

window.addEventListener("pagehide", () => { encerrar(); pararNavegador(); });

atualizarBotaoSom();
atualizarLista();
setInterval(atualizarLista, INTERVALO_LISTA_MS);

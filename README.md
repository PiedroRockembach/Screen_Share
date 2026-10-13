# Sala de Tela

Substituto gratuito do "Go Live" do Discord: transmissão de tela em 1080p a 60 fps
nos dois sentidos (você no CachyOS, amigos no Windows 11), pela VPN do Tailscale,
**sem a voz do Discord no áudio** (nada de eco na chamada).

```
Seu PC (servidor)                               Amigos (Windows)
OBS ──WHIP──► MediaMTX ◄──WHIP── "Transmitir minha tela" (GStreamer)
              MediaMTX ──WHEP──► "Sala de Tela" (navegador)
```

| Peça | Onde | O que faz |
|---|---|---|
| `servidor/setup-linux.sh` | seu PC | instala e configura tudo do seu lado |
| `servidor/mediamtx.yml` | seu PC | servidor WebRTC (recebe WHIP, entrega WHEP) |
| `servidor/sala_web.py` + `web/` | seu PC | página da Sala (porta 8080) |
| `servidor/sala_audio.py` | seu PC | põe todo app na saída "Sala", menos o Discord |
| `servidor/gerar_obs.py` | seu PC | perfil do OBS: NVENC 1080p60, WHIP, só o áudio da "Sala" |
| `windows/build-instalador.sh` | seu PC / GitHub | gera o `dist/Instalar-Sala.bat` genérico (sem segredos) |
| `windows/gerar-convite.sh` | seu PC | gera o código de convite (IP + chave da VPN) de cada amigo |
| `windows/Instalar-Sala.ps1` | amigos | instala Tailscale + GStreamer e cria os atalhos |
| `windows/Transmitir.ps1` | amigos | janela "Transmitir": tela + som do PC sem o Discord |

Tudo é gratuito: OBS (GPL), MediaMTX (MIT), GStreamer (LGPL), PipeWire (MIT) e o
plano Personal do Tailscale (os amigos entram por chave com tag, sem conta).

## 1. Seu lado (uma vez)

```bash
./servidor/setup-linux.sh piedro      # "piedro" = nome da sua transmissão
```

Ele pede sua senha do sudo e abre o login do Tailscale na primeira vez. No fim, mostra o IP da VPN.

## 2. Tailscale (uma vez, no navegador)

Em **Access controls**, cole `windows/tailscale-acl.hujson`, trocando `IP_DO_SERVIDOR`
pelo seu IP da VPN. Com isso, os amigos só alcançam as portas da Sala no seu PC.

## 3. Publicar no GitHub e convidar amigos

O instalador é **genérico**: não tem chave nem IP, então o repositório pode ser público.
A cada push que mexe em `windows/`, o GitHub Actions gera o `.bat` e publica num link fixo:

```
https://github.com/<usuario>/<repo>/releases/download/instalador/Instalar-Sala.bat
```

Para cada amigo:

1. Em **Settings → Keys → Generate auth key**, crie uma chave **de uso único**
   (Reusable desmarcado), **Pre-approved**, tag **tag:amigo**, validade de 1 dia.
2. Rode `./windows/gerar-convite.sh` e cole a chave. Ele imprime um código `SALA-…`.
3. Mande o link do instalador e, **em mensagem privada**, o código.
   O instalador pede o código uma vez só.

Depois de usado, o código não serve para mais nada. Se vazar antes, quem usar só
alcança as portas da Sala (a ACL bloqueia o resto). Para apagar um PC da sua VPN,
use **Machines** no painel do Tailscale. Para gerar o `.bat` localmente sem o GitHub:
`./windows/build-instalador.sh`.

## Uso

- **Transmitir:** menu de aplicativos → **Sala de Tela: Transmitir** (abre o OBS já ao vivo).
  Na primeira vez, o KDE pergunta qual tela compartilhar.
- **Assistir:** **Sala de Tela: Assistir**, ou `http://127.0.0.1:8080/`.
- **Conferir se o Discord está fora do áudio:** `python3 servidor/sala_audio.py status`

O seu PC precisa estar ligado para a Sala funcionar, porque ele é o servidor.

## Como o eco é evitado

- **Linux:** a saída padrão continua sendo o seu fone ou caixa. O `sala_audio.py` move todo app,
  menos o Discord, para a saída virtual "Sala", que repassa o som para o fone. O OBS captura
  só a "Sala". Se algo falhar, o pior caso é um jogo ficar sem som na transmissão.
  A voz do Discord nunca entra.
- **Windows:** o GStreamer captura o som do PC **excluindo a árvore de processos do Discord**
  (`wasapi2src loopback-mode=exclude-process-tree`, Windows build 20348 ou mais recente).
  Sem o app do Discord aberto, ou com mais de um aberto, a transmissão vai **sem som**.
  Se o Discord abrir ou fechar durante a transmissão, ela reinicia em até 1 segundo
  com o processo certo excluído.

## Fase 0: teste com um amigo (fazer antes de usar de verdade)

A exclusão do Discord no Windows ainda não foi testada num PC real. Teste assim:

1. O amigo instala, abre **Transmitir minha tela** e clica em **Diagnóstico**.
   Confira no arquivo se aparecem `whipclientsink ok`, `d3d11screencapturesrc ok`
   e `loopback-mode: disponível`.
2. Vocês entram numa chamada do Discord e o amigo começa a transmitir com um vídeo do YouTube tocando.
3. Você assiste pela Sala **com fone** e pede para alguém falar na chamada.
   - O som do vídeo aparece na transmissão e a voz da chamada **não**: tudo certo.
   - A voz aparece na transmissão: gere convites só com vídeo
     (`./windows/gerar-convite.sh --sem-audio`). Os amigos apagam
     `C:\ProgramData\SalaDeTela\servidor.json` e reinstalam com o código novo.
4. Confira a qualidade em `chrome://webrtc-internals`: 1920x1080, cerca de 60 fps, H264.

## Problemas comuns

| Sintoma | O que fazer |
|---|---|
| Amigo não abre a Sala | `tailscale status` (ele aparece?), `systemctl --user status sala-web sala-mediamtx` |
| Imagem trava ou está lenta | `tailscale status`: a conexão deve ser `direct`, não `relay` |
| Meu jogo sem som na transmissão | `python3 servidor/sala_audio.py status` e `systemctl --user status sala-audio` |
| Amigo com erro ao transmitir | peça o arquivo do botão **Diagnóstico** |

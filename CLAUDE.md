# CLAUDE.md

## Contexto pessoal (piedro-os)
@~/Documentos/piedro-os/contextos/pessoal.md
@~/Documentos/piedro-os/projetos/sala-de-tela.md

## Projeto (para sessões sem o piedro-os, como no Windows)
Substituto gratuito do "Go Live" do Discord: transmissão de tela 1080p60 entre amigos, com o som do PC **sem a voz do Discord**. Fale sempre em português do Brasil; código, identificadores e commits em inglês (Conventional Commits). Nunca faça commit ou push sem pedido explícito.

Restrições fixas: 100% gratuito; sem Tailscale nem serviços de terceiros na rede; "instalou, usou" para amigos leigos; saída fixa em 1080p60; se não der para tirar o Discord do áudio com segurança, transmitir sem som.

- `desktop/` é o app atual (Rust): Slint na interface, gstreamer-rs para captura, codificação e reprodução, MediaMTX como servidor de vídeo (SRT em 127.0.0.1:48890, API em 48891).
  - `tunnel.rs`: túnel UDP próprio na porta 48900; o convidado prova o segredo da sala por HMAC; cada conexão SRT vira um fluxo.
  - `room.rs`: convite `sala1-…` (segredo + IPv4 da rede de casa + IPv6 público estável).
  - `sender_windows.rs`: captura D3D11, escolha do codificador e áudio `wasapi2src loopback-mode=exclude-process-tree` excluindo o processo raiz do Discord (verificado a cada 1 s).
  - `player.rs`: srtsrc → tsdemux (latency=0) → decodificador → appsink desenhado na janela.
- `app/` é o app antigo em Go (Funnel + TURN), mantido até o novo substituir.

### Pacotes Linux
- `.github/workflows/linux.yml` gera o `.deb` (compilado em Debian 12, release `app-debian`) e o `.rpm` (Fedora, release `app-fedora`); arquivos em `desktop/packaging/`. MediaMTX fixado em v1.21.4 (a v1.21.2 descartava quadros H.264 grandes em silêncio).

### No Windows
- O instalador sai do workflow `.github/workflows/desktop.yml` (GStreamer 1.28.8 MSVC em `C:\gstreamer`, script `desktop/installer/bundle.ps1`, Inno Setup).
- Arquivos do app instalado: `%LOCALAPPDATA%\SalaDeTela` (`desktop.json`, `desktop-mediamtx.log`, `gstreamer-registry.bin`).
- Para compilar localmente: instalar o GStreamer MSVC (runtime + desenvolvimento), `pkg-config` e Rust; `PKG_CONFIG_PATH=C:\gstreamer\lib\pkgconfig`; `cargo build --release` em `desktop/`.
- Para depurar o GStreamer: rodar com `GST_DEBUG=3` e conferir elementos com `gst-inspect-1.0 <elemento>`.

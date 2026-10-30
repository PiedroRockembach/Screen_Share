# Sala de Tela

Substituto gratuito do "Go Live" do Discord: compartilhamento de tela em 1080p a 60 fps
entre amigos, com o som do PC **sem a voz do Discord** (sem eco na chamada).

Um único programa para **Windows** e **Linux**. Qualquer pessoa pode criar a sala;
os convidados entram por um **link**, o mesmo para todos e o mesmo todos os dias.

## Baixar

| Sistema | Arquivo |
|---|---|
| Windows 10/11 | [`SalaDeTela.exe`](../../releases/download/ultima/SalaDeTela.exe) |
| Linux | [`sala-de-tela-linux.tar.gz`](../../releases/download/ultima/sala-de-tela-linux.tar.gz) (extraia e abra `sala-de-tela`) |

O GitHub Actions compila e publica esses arquivos a cada push em `main`.

## Como usar

Abra o programa com dois cliques. O painel aparece no navegador.

- **Criar uma sala:** clique em **Criar sala**. Na primeira vez:
  1. o programa instala o Tailscale, se precisar (no Linux, ele pede a senha do sistema);
  2. você entra na sua conta do Tailscale, que é gratuita e pode ser com o Google;
  3. você autoriza o "Funnel" numa página do Tailscale (um clique).

  Depois que o endereço aparecer no DNS público (isso pode levar até 10 minutos
  na primeira configuração), aparece o **link de convite**. Mande ele para os amigos.
  A sala fica aberta enquanto o programa estiver aberto.
- **Assistir:** abra o link no navegador. Não precisa instalar nada.
- **Transmitir a sua tela:**
  - **Com som:** no programa, cole o link da sala (ou deixe vazio para a sua própria sala),
    escreva o seu nome e clique em **Transmitir**.
  - **Sem som, sem instalar nada:** na página da sala, clique em **Transmitir minha tela**.

## Como funciona

```
Quem cria a sala                                      Convidados
 Sala de Tela ── MediaMTX (WebRTC)                    navegador / Sala de Tela
      │                 ▲   ▲ vídeo P2P (UDP 47804) ──────┘
      └── Tailscale Funnel: https://<pc>.ts.net/s/<código>/  ◄── página, WHIP, WHEP
```

- **Funnel:** publica só a página e a negociação (WHIP/WHEP) num HTTPS público e fixo.
  Sem o `<código>` certo, o servidor responde 404. **Trocar o link** gera um código novo
  e o antigo para de funcionar.
- **Vídeo:** vai **direto** entre os PCs pela internet (WebRTC com STUN). Não passa
  pelo Tailscale e não depende de VPN.
- **Transmitir no Windows:** GStreamer, instalado sozinho na primeira vez (~540 MB).
  Ele captura a tela, usa o codificador de vídeo da placa (NVIDIA/AMD/Intel) e pega o som
  do PC **excluindo os processos do Discord** (Windows 11). Sem o app do Discord aberto,
  vai sem som. Se o Discord abrir ou fechar, a transmissão reinicia em até 1 s.
- **Transmitir no Linux:** OBS, com um perfil "Sala de Tela" gerado pelo programa.
  Enquanto você transmite, todos os apps tocam numa saída virtual "sala_de_tela",
  menos o Discord, e o OBS captura só essa saída. Ao parar, tudo volta ao normal.

Tudo é gratuito: Tailscale (plano pessoal), MediaMTX, GStreamer, OBS e os servidores STUN públicos.

## Limitações conhecidas

- **Redes muito restritas:** como o vídeo vai direto entre os PCs, algumas redes (certos
  4G/CGNAT ou redes de empresa) podem não conectar. Não há servidor de retransmissão gratuito.
- **Windows sem testes:** a transmissão no Windows ainda não foi testada num PC real. Antes
  de usar de verdade, faça um teste numa chamada: a voz do Discord **não** pode aparecer
  na transmissão. Se aparecer, desmarque "Transmitir o som do PC".
- **Aviso do Windows:** o SmartScreen avisa na primeira vez porque o programa não é assinado.
  Clique em "Mais informações" e depois em "Executar assim mesmo".
- **Linux:** precisa do OBS Studio instalado para transmitir.

## Desenvolvimento

```bash
go vet ./app && GOOS=windows go vet ./app
go build -o dist/sala-de-tela ./app
SALA_DEV=1 ./dist/sala-de-tela   # sem Tailscale/Funnel, sala só em 127.0.0.1, não abre o navegador
```

Arquivos e logs ficam em `~/.local/share/sala-de-tela` (Linux) e `%LOCALAPPDATA%\SalaDeTela` (Windows).

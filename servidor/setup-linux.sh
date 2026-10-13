#!/usr/bin/env bash
# Sala de Tela: instala e configura o lado do servidor (CachyOS/Arch).
# Pode ser rodado de novo quantas vezes quiser (idempotente).
#
# Uso: ./setup-linux.sh [nome-da-sua-transmissao]
set -euo pipefail

MEDIAMTX_VERSAO="v1.21.2"
PROJETO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVIDOR="$PROJETO/servidor"
DADOS="$HOME/.local/share/sala-de-tela"
UNITS="$HOME/.config/systemd/user"

passo() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
ok()    { printf '    \033[32m✔\033[0m %s\n' "$*"; }
aviso() { printf '    \033[33m!\033[0m %s\n' "$*"; }

[[ $EUID -ne 0 ]] || { echo "Rode como seu usuário normal (sem sudo); o script pede a senha quando precisar."; exit 1; }

# ---------------------------------------------------------------- nome
NOME="${1:-${USER}}"
NOME="$(printf '%s' "$NOME" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-' | sed 's/^-*//; s/-*$//')"
[[ -n "$NOME" ]] || { echo "Nome inválido."; exit 1; }
echo "Sua transmissão vai se chamar: $NOME"

# ---------------------------------------------------------------- pacotes
passo "Pacotes (tailscale, obs-studio)"
sudo pacman -S --needed --noconfirm tailscale obs-studio libpulse python curl
ok "pacotes instalados"

# ---------------------------------------------------------------- tailscale
passo "Tailscale (VPN)"
sudo systemctl enable --now tailscaled
if ! tailscale ip -4 >/dev/null 2>&1; then
    aviso "Faça login no Tailscale no navegador (só desta vez)."
    sudo tailscale up --operator="$USER"
fi
sudo tailscale set --operator="$USER" >/dev/null
IP_TS="$(tailscale ip -4 | head -1)"
ok "conectado à VPN com o IP $IP_TS"

# ---------------------------------------------------------------- mediamtx
passo "MediaMTX $MEDIAMTX_VERSAO"
mkdir -p "$DADOS/bin"
if [[ "$("$DADOS/bin/mediamtx" --version 2>/dev/null || true)" != "$MEDIAMTX_VERSAO" ]]; then
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT
    ARQ="mediamtx_${MEDIAMTX_VERSAO}_linux_amd64.tar.gz"
    URL="https://github.com/bluenviron/mediamtx/releases/download/$MEDIAMTX_VERSAO"
    curl -fsSL -o "$TMP/$ARQ" "$URL/$ARQ"
    curl -fsSL -o "$TMP/checksums.sha256" "$URL/checksums.sha256"
    (cd "$TMP" && grep -E " \*?$ARQ\$" checksums.sha256 | sha256sum -c --quiet -)
    tar -xzf "$TMP/$ARQ" -C "$TMP" mediamtx
    install -m 755 "$TMP/mediamtx" "$DADOS/bin/mediamtx"
fi
ok "$("$DADOS/bin/mediamtx" --version) em $DADOS/bin"

# ---------------------------------------------------------------- firewall
passo "Firewall (portas abertas só dentro da VPN)"
if command -v ufw >/dev/null && sudo ufw status | grep -q "Status: active"; then
    sudo ufw allow in on tailscale0 to any port 8080 proto tcp comment "Sala de Tela: pagina" >/dev/null
    sudo ufw allow in on tailscale0 to any port 8889 proto tcp comment "Sala de Tela: WebRTC" >/dev/null
    sudo ufw allow in on tailscale0 to any port 8189 proto udp comment "Sala de Tela: midia" >/dev/null
    sudo ufw allow 41641/udp comment "Tailscale: conexao direta" >/dev/null
    ok "ufw liberado em tailscale0 (8080/tcp, 8889/tcp, 8189/udp)"
else
    aviso "ufw não está ativo: nada a fazer (as portas só são alcançáveis pela VPN se o roteador não as encaminhar)"
fi

# ---------------------------------------------------------------- áudio
passo "Áudio: saída virtual 'Sala' (sem Discord)"
PW_CONF="$HOME/.config/pipewire/pipewire.conf.d/60-sala.conf"
mkdir -p "$(dirname "$PW_CONF")"
if ! cmp -s "$SERVIDOR/pipewire/60-sala.conf" "$PW_CONF"; then
    install -m 644 "$SERVIDOR/pipewire/60-sala.conf" "$PW_CONF"
    aviso "reiniciando o PipeWire (o som some por 1 segundo)"
    systemctl --user restart pipewire pipewire-pulse wireplumber
    sleep 2
fi
pactl list short sinks | grep -q sala_sink && ok "saída 'Sala' ativa" || aviso "saída 'Sala' não apareceu; veja: journalctl --user -u pipewire"

# ---------------------------------------------------------------- serviços
passo "Serviços (iniciam junto com o seu login)"
mkdir -p "$UNITS"
cat > "$UNITS/sala-mediamtx.service" <<EOF
[Unit]
Description=Sala de Tela: servidor WebRTC (MediaMTX)
After=network-online.target

[Service]
ExecStart=$DADOS/bin/mediamtx $SERVIDOR/mediamtx.yml
Restart=on-failure

[Install]
WantedBy=default.target
EOF
cat > "$UNITS/sala-web.service" <<EOF
[Unit]
Description=Sala de Tela: página web
After=sala-mediamtx.service

[Service]
ExecStart=/usr/bin/python3 $SERVIDOR/sala_web.py --porta 8080 --web $PROJETO/web
Restart=on-failure

[Install]
WantedBy=default.target
EOF
cat > "$UNITS/sala-audio.service" <<EOF
[Unit]
Description=Sala de Tela: mantém o Discord fora do áudio transmitido
After=pipewire-pulse.service
Wants=pipewire-pulse.service

[Service]
ExecStart=/usr/bin/python3 $SERVIDOR/sala_audio.py rodar
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
EOF
systemctl --user daemon-reload
systemctl --user enable sala-mediamtx sala-web sala-audio >/dev/null 2>&1
systemctl --user restart sala-mediamtx sala-web sala-audio
sleep 1
for s in sala-mediamtx sala-web sala-audio; do
    systemctl --user is-active --quiet "$s" && ok "$s rodando" || aviso "$s falhou: journalctl --user -u $s"
done

# ---------------------------------------------------------------- OBS
passo "OBS: perfil 'Sala de Tela'"
python3 "$SERVIDOR/gerar_obs.py" --nome "$NOME" >/dev/null
mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/sala-de-tela-transmitir.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Sala de Tela: Transmitir
Comment=Abre o OBS já transmitindo sua tela para a Sala
Exec=obs --profile "Sala de Tela" --collection "Sala de Tela" --startstreaming
Icon=com.obsproject.Studio
Categories=AudioVideo;
EOF
cat > "$HOME/.local/share/applications/sala-de-tela-assistir.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Sala de Tela: Assistir
Comment=Assistir a tela dos amigos
Exec=xdg-open http://127.0.0.1:8080/
Icon=video-display
Categories=AudioVideo;
EOF
ok "atalhos 'Sala de Tela: Transmitir' e 'Sala de Tela: Assistir' no menu"

# ---------------------------------------------------------------- resumo
echo "$IP_TS" > "$DADOS/ip-servidor"
passo "Pronto!"
cat <<EOF
    Página da sala (para você):   http://127.0.0.1:8080/
    Página da sala (para amigos): http://$IP_TS:8080/
    Sua transmissão:              /$NOME  (abra "Sala de Tela: Transmitir" no menu)

    Próximos passos (uma vez só):
      1. No painel do Tailscale, cole a ACL de windows/tailscale-acl.hujson
         (troque IP_DO_SERVIDOR por $IP_TS)
      2. Suba o projeto no GitHub: o Actions publica o Instalar-Sala.bat
      3. Para cada amigo: ./windows/gerar-convite.sh   (código em mensagem privada)

    Conferir o áudio sem Discord a qualquer momento:
      python3 $SERVIDOR/sala_audio.py status
EOF

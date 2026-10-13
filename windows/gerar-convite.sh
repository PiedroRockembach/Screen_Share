#!/usr/bin/env bash
# Gera um código de convite para UM amigo. Mande por mensagem privada.
# O código leva o IP do servidor, uma auth key do Tailscale e a opção de áudio.
#
# Use uma chave de USO ÚNICO por amigo (Reusable desmarcado, Pre-approved,
# tag:amigo, validade curta, ex.: 1 dia). Depois que ele instala, o código não
# serve para mais nada; e mesmo antes, a ACL só libera as portas da Sala.
#
# Uso: ./windows/gerar-convite.sh [--sem-audio]
set -euo pipefail

AUDIO=true
[[ "${1:-}" == "--sem-audio" ]] && AUDIO=false

IP="${SALA_IP:-$(tailscale ip -4 2>/dev/null | head -1 || true)}"
[[ -n "$IP" ]] || IP="$(cat "$HOME/.local/share/sala-de-tela/ip-servidor" 2>/dev/null || true)"
[[ "$IP" =~ ^100\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Não achei o IP do Tailscale. Rode o servidor/setup-linux.sh primeiro."; exit 1; }

KEY="${SALA_AUTHKEY:-}"
if [[ -z "$KEY" ]]; then
    echo "Gere uma auth key de USO ÚNICO com a tag tag:amigo e cole aqui:"
    echo "  https://login.tailscale.com/admin/settings/keys"
    read -rsp "> " KEY; echo
fi
[[ "$KEY" =~ ^tskey-auth-[A-Za-z0-9_-]+$ ]] || { echo "Isso não parece uma auth key (deveria começar com tskey-auth-)."; exit 1; }

CODIGO="SALA-$(printf '%s|%s|%s' "$IP" "$KEY" "$AUDIO" | base64 -w0)"
echo
echo "Código de convite (servidor $IP, áudio: $AUDIO). Mande em mensagem PRIVADA:"
echo
echo "$CODIGO"

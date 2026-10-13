#!/usr/bin/env bash
# Gera dist/Instalar-Sala.bat: um único arquivo, GENÉRICO, que pode ficar
# público (GitHub). Ele não tem chave nem IP: isso vem no código de convite
# (windows/gerar-convite.sh), mandado em mensagem privada para cada amigo.
#
# Uso: ./windows/build-instalador.sh
# (o GitHub Actions roda isto sozinho a cada push; veja .github/workflows)
set -euo pipefail

PROJETO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WIN="$PROJETO/windows"
DIST="$PROJETO/dist"

# Transmitir.ps1 vai embutido em base64; o instalador preenche IP e áudio na hora.
TRANSMITIR_B64="$(base64 -w0 "$WIN/Transmitir.ps1")"
INSTALADOR="$(sed -e "s|__TRANSMITIR_B64__|$TRANSMITIR_B64|" "$WIN/Instalar-Sala.ps1")"

mkdir -p "$DIST"
SAIDA="$DIST/Instalar-Sala.bat"
{
    # Cabeçalho em batch: pede administrador e roda o PowerShell que vem depois do marcador.
    cat <<'BAT'
@echo off
title Sala de Tela - instalador
fltmc >nul 2>&1
if %errorlevel% neq 0 (
  powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
  exit /b
)
set "SALA_BAT=%~f0"
powershell -NoProfile -ExecutionPolicy Bypass -Command "$t=[IO.File]::ReadAllText($env:SALA_BAT,[Text.Encoding]::UTF8); $i=$t.LastIndexOf('#SALA-PS1'); Invoke-Expression $t.Substring($i)"
exit /b
BAT
    echo "#SALA-PS1"
    printf '%s\n' "$INSTALADOR"
} | sed 's/$/\r/' > "$SAIDA"   # .bat precisa de CRLF

# Segurança: o instalador público nunca pode ter uma chave dentro.
if grep -oE 'tskey-auth-[A-Za-z0-9]{5,}' "$SAIDA" >/dev/null; then echo "ERRO: há uma chave do Tailscale no instalador!"; exit 1; fi
if grep -q '__TRANSMITIR_B64__' "$SAIDA"; then echo "ERRO: marcador não substituído em $SAIDA"; exit 1; fi

echo "Gerado: $SAIDA (genérico, sem segredos)"

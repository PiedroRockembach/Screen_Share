#!/usr/bin/env python3
"""Mantém a voz do Discord fora da transmissão.

Move todo app que toca na saída padrão para a saída virtual "Sala" (que o OBS
captura), menos o Discord e similares. Se o Discord for parar na "Sala",
ele volta para a saída padrão na hora.

Uso:
  sala_audio.py rodar    # serviço: reage a cada novo som (pactl subscribe)
  sala_audio.py status   # mostra para onde cada app está indo
"""

import json
import re
import subprocess
import sys
import time

SALA = "sala_sink"
DISCORD = re.compile(r"discord|vesktop|webcord|legcord|armcord", re.IGNORECASE)


def pactl(*args):
    return subprocess.run(["pactl", *args], capture_output=True, text=True, check=False)


def pactl_json(*args):
    saida = pactl("-f", "json", *args).stdout
    return json.loads(saida) if saida.strip() else []


def nomes_dos_sinks():
    return {s["index"]: s["name"] for s in pactl_json("list", "sinks")}


def processos_ancestrais(pid):
    """Executáveis do processo e de todos os pais (para pegar filhos do Discord)."""
    vistos = []
    while pid and pid > 1 and len(vistos) < 32:
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as f:
                argv0 = f.read().split(b"\0", 1)[0].decode(errors="replace")
            vistos.append(argv0.rsplit("/", 1)[-1])
            with open(f"/proc/{pid}/stat") as f:
                # o nome do processo pode ter espaços e parênteses: o ppid vem depois do último ')'
                pid = int(f.read().rsplit(")", 1)[1].split()[1])
        except (OSError, ValueError, IndexError):
            break
    return vistos


def eh_discord(props):
    for chave, valor in props.items():
        if (chave.startswith("application.") or chave == "pipewire.access.portal.app_id") \
                and DISCORD.search(str(valor)):
            return True
    try:
        pid = int(props.get("application.process.id", "0"))
    except ValueError:
        pid = 0
    return any(DISCORD.search(cmd) for cmd in processos_ancestrais(pid))


def ignorar(props):
    """Loopbacks, filtros e a própria Sala não são apps: não mexer."""
    nome = props.get("node.name", "")
    return nome.startswith("sala_") or "node.link-group" in props


def ajustar(mostrar=False):
    sinks = nomes_dos_sinks()
    sala = next((i for i, n in sinks.items() if n == SALA), None)
    padrao = pactl("get-default-sink").stdout.strip()
    if sala is None:
        if mostrar:
            print(f"ALERTA: a saída '{SALA}' não existe (rode o setup-linux.sh).")
        return
    if padrao == SALA:
        if mostrar:
            print("ALERTA: a 'Sala' virou a saída padrão. Escolha seu fone/caixa como padrão.")
        return

    for entrada in pactl_json("list", "sink-inputs"):
        props = entrada.get("properties", {})
        if ignorar(props):
            continue
        app = props.get("application.name") or props.get("node.name") or "?"
        onde = sinks.get(entrada["sink"], "?")
        discord = eh_discord(props)

        if discord and onde == SALA:
            pactl("move-sink-input", str(entrada["index"]), padrao)
            onde = padrao
        elif not discord and onde == padrao:
            pactl("move-sink-input", str(entrada["index"]), SALA)
            onde = SALA

        if mostrar:
            if discord:
                situacao = "ALERTA: Discord na transmissão!" if onde == SALA else "fora da transmissão (Discord) ✔"
            else:
                situacao = "na transmissão ✔" if onde == SALA else f"fora da transmissão (saída {onde})"
            print(f"{app:30} {situacao}")


def rodar():
    ajustar()
    while True:
        eventos = subprocess.Popen(["pactl", "subscribe"], stdout=subprocess.PIPE, text=True)
        for linha in eventos.stdout:
            if "sink-input" in linha or "'server'" in linha or "on sink #" in linha:
                ajustar()
        # pactl saiu (PipeWire reiniciou?): espera e tenta de novo
        eventos.wait()
        time.sleep(2)
        ajustar()


def main():
    comando = sys.argv[1] if len(sys.argv) > 1 else "status"
    if comando == "rodar":
        rodar()
    elif comando == "status":
        ajustar(mostrar=True)
    else:
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Servidor da página da Sala de Tela.

Serve os arquivos de web/ e expõe /api/ao-vivo, uma versão somente leitura
da lista de transmissões do MediaMTX (a API completa fica restrita ao localhost).
"""

import argparse
import json
import urllib.request
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer

API_MEDIAMTX = "http://127.0.0.1:9997/v3/paths/list"


def transmissoes_ao_vivo():
    with urllib.request.urlopen(API_MEDIAMTX, timeout=3) as resp:
        dados = json.load(resp)
    ao_vivo = []
    for item in dados.get("items", []):
        if not item.get("ready"):
            continue
        ao_vivo.append({
            "nome": item["name"],
            "faixas": item.get("tracks", []),
            "espectadores": len(item.get("readers", [])),
        })
    return sorted(ao_vivo, key=lambda t: t["nome"])


class Handler(SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.split("?")[0] == "/api/ao-vivo":
            try:
                corpo = json.dumps(transmissoes_ao_vivo()).encode()
                status = 200
            except OSError:
                corpo = b'{"erro": "MediaMTX fora do ar"}'
                status = 503
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(corpo)))
            self.end_headers()
            self.wfile.write(corpo)
            return
        super().do_GET()

    def end_headers(self):
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()

    def log_message(self, fmt, *args):
        pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--porta", type=int, default=8080)
    parser.add_argument("--web", required=True, help="pasta com index.html")
    args = parser.parse_args()

    servidor = ThreadingHTTPServer(("0.0.0.0", args.porta), partial(Handler, directory=args.web))
    print(f"Sala de Tela em http://0.0.0.0:{args.porta}/", flush=True)
    servidor.serve_forever()


if __name__ == "__main__":
    main()

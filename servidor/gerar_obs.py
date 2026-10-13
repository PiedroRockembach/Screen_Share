#!/usr/bin/env python3
"""Cria o perfil e a coleção de cenas "Sala de Tela" no OBS (testado no OBS 32).

- Vídeo: tela inteira (PipeWire) redimensionada para 1920x1080 a 60 fps
- Codificador: NVENC H.264, CBR, keyframe a cada 1 s, sem B-frames
- Áudio: SÓ o monitor da saída "Sala" (sem microfone e sem Discord)
- Saída: WHIP para o MediaMTX local

Não mexe nos perfis e cenas que já existem.
"""

import argparse
import json
import uuid
from pathlib import Path

NOME = "Sala de Tela"
PASTA = "SalaDeTela"


def escrever(caminho: Path, conteudo: str):
    caminho.parent.mkdir(parents=True, exist_ok=True)
    caminho.write_text(conteudo, encoding="utf-8")


def perfil(base: Path, caminho_whip: str, bitrate: int):
    pasta = base / "basic" / "profiles" / PASTA
    escrever(pasta / "basic.ini", f"""[General]
Name={NOME}

[Output]
Mode=Advanced

[AdvOut]
Encoder=obs_nvenc_h264_tex
AudioEncoder=ffmpeg_opus
TrackIndex=1
RecEncoder=none

[Video]
BaseCX=1920
BaseCY=1080
OutputCX=1920
OutputCY=1080
FPSType=0
FPSCommon=60
ScaleType=bicubic
ColorFormat=NV12
ColorSpace=709
ColorRange=Partial

[Audio]
SampleRate=48000
ChannelSetup=Stereo
""")
    escrever(pasta / "streamEncoder.json", json.dumps({
        "rate_control": "CBR",
        "bitrate": bitrate,
        "keyint_sec": 1,
        "preset2": "p5",
        "tune": "ll",
        "multipass": "disabled",
        "profile": "high",
        "bf": 0,
        "lookahead": False,
    }, indent=2))
    escrever(pasta / "service.json", json.dumps({
        "type": "whip_custom",
        "settings": {
            "service": "WHIP",
            "server": f"http://127.0.0.1:8889/{caminho_whip}/whip",
            "bearer_token": "",
        },
    }, indent=2))


def colecao(base: Path):
    id_tela = str(uuid.uuid4())
    id_cena = str(uuid.uuid4())
    id_audio = str(uuid.uuid4())

    tela = {
        "id": "pipewire-screen-capture-source",
        "versioned_id": "pipewire-screen-capture-source",
        "name": "Minha tela",
        "uuid": id_tela,
        "settings": {"ShowCursor": True},
        "enabled": True,
    }
    cena = {
        "id": "scene",
        "versioned_id": "scene",
        "name": "Sala",
        "uuid": id_cena,
        "settings": {
            "id_counter": 1,
            "custom_size": False,
            "items": [{
                "name": "Minha tela",
                "source_uuid": id_tela,
                "visible": True,
                "locked": True,
                "rot": 0.0,
                "align": 5,
                # Escala a tela (4K, ultrawide, etc.) para caber em 1920x1080.
                "bounds_type": 2,
                "bounds_align": 0,
                "bounds": {"x": 1920.0, "y": 1080.0},
                "pos": {"x": 0.0, "y": 0.0},
                "scale": {"x": 1.0, "y": 1.0},
                "id": 1,
            }],
        },
        "enabled": True,
    }
    audio = {
        "id": "pulse_output_capture",
        "versioned_id": "pulse_output_capture",
        "name": "Áudio da Sala (sem Discord)",
        "uuid": id_audio,
        "settings": {"device_id": "sala_sink.monitor"},
        "mixers": 255,
        "volume": 1.0,
        "enabled": True,
        "muted": False,
        "monitoring_type": 0,
    }
    dados = {
        "name": NOME,
        # Só a saída "Sala" como áudio do desktop; sem microfone (AuxAudioDevice).
        "DesktopAudioDevice1": audio,
        "sources": [tela, cena],
        "groups": [],
        "scene_order": [{"name": "Sala"}],
        "current_scene": "Sala",
        "current_program_scene": "Sala",
        "transitions": [],
        "current_transition": "Fade",
        "transition_duration": 300,
    }
    escrever(base / "basic" / "scenes" / f"{PASTA}.json", json.dumps(dados, indent=2, ensure_ascii=False))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--nome", required=True, help="nome da sua transmissão (caminho no MediaMTX)")
    parser.add_argument("--bitrate", type=int, default=9000, help="kbit/s")
    parser.add_argument("--config", default=str(Path.home() / ".config" / "obs-studio"))
    args = parser.parse_args()

    base = Path(args.config)
    perfil(base, args.nome, args.bitrate)
    colecao(base)
    print(f"OBS: perfil e cenas '{NOME}' criados em {base}")


if __name__ == "__main__":
    main()

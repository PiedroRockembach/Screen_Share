package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

const (
	portaWebRTC = 47802 // WHIP/WHEP do MediaMTX, só 127.0.0.1 (o proxy da sala repassa)
	portaAPI    = 47803 // API do MediaMTX, só 127.0.0.1
	portaMidia  = 47804 // UDP: o vídeo em si, direto entre os PCs (P2P)
)

// Só WebRTC, tudo atrás do proxy da sala (que confere o código do link).
// O STUN descobre o IP público para o vídeo ir direto entre os PCs.
const configMediaMTX = `
logLevel: info
rtsp: false
rtmp: false
hls: false
srt: false
moq: false

webrtc: true
webrtcAddress: 127.0.0.1:%d
webrtcLocalUDPAddress: :%d
webrtcLocalTCPAddress: ""
webrtcICEServers2:
  - url: stun:stun.l.google.com:19302
  - url: stun:stun.cloudflare.com:3478

api: true
apiAddress: 127.0.0.1:%d

authInternalUsers:
  - user: any
    pass:
    ips: ["127.0.0.1", "::1"]
    permissions:
      - action: publish
      - action: read
      - action: api

paths:
  all_others:
`

func binarioMediaMTX() string {
	nome := "mediamtx"
	if runtime.GOOS == "windows" {
		nome += ".exe"
	}
	return filepath.Join(pastaDados, "bin", nome)
}

// garantirMediaMTX baixa o MediaMTX oficial (com checksum) se ainda não tiver.
func (a *App) garantirMediaMTX() error {
	bin := binarioMediaMTX()
	if saida, err := exec.Command(bin, "--version").Output(); err == nil && string(saida) == versaoMediaMTX+"\n" {
		return nil
	}
	ext := "tar.gz"
	if runtime.GOOS == "windows" {
		ext = "zip"
	}
	arq := fmt.Sprintf("mediamtx_%s_%s_%s.%s", versaoMediaMTX, runtime.GOOS, runtime.GOARCH, ext)
	base := "https://github.com/bluenviron/mediamtx/releases/download/" + versaoMediaMTX
	pacote := filepath.Join(pastaDados, "downloads", arq)
	a.registrar("baixando o MediaMTX %s…", versaoMediaMTX)
	if err := a.baixarVerificado(base+"/"+arq, base+"/checksums.sha256", pacote); err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(bin), 0o755); err != nil {
		return err
	}
	if err := extrairDe(pacote, filepath.Base(bin), bin); err != nil {
		return err
	}
	os.Remove(pacote)
	return nil
}

func iniciarMediaMTX() (*exec.Cmd, error) {
	conf := filepath.Join(pastaDados, "mediamtx.yml")
	if err := os.WriteFile(conf, []byte(fmt.Sprintf(configMediaMTX, portaWebRTC, portaMidia, portaAPI)), 0o644); err != nil {
		return nil, err
	}
	cmd := exec.Command(binarioMediaMTX(), conf)
	cmd.Dir = pastaDados // o MediaMTX cria arquivos auxiliares na pasta atual
	logArq, err := os.Create(filepath.Join(pastaDados, "mediamtx.log"))
	if err == nil {
		cmd.Stdout, cmd.Stderr = logArq, logArq
	}
	prepararFilho(cmd)
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("iniciar MediaMTX: %w", err)
	}
	vincularFilho(cmd)
	return cmd, nil
}

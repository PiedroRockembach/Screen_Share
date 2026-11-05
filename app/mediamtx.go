package main

import (
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
)

const (
	portaWebRTC = 47802 // WHIP/WHEP do MediaMTX, só 127.0.0.1 (o proxy da sala repassa)
	portaAPI    = 47803 // API do MediaMTX, só 127.0.0.1
	portaMidia  = 47804 // UDP: o vídeo em si, vindo da retransmissão (TURN)
)

// Só WebRTC, tudo atrás do proxy da sala (que confere o código do link).
// O vídeo entre este PC e os convidados passa pela retransmissão (TURN) da sala,
// que o Funnel publica por HTTPS. Por isso o MediaMTX só anuncia o IP por onde o
// TURN entrega os pacotes, escuta só nele e não usa STUN: com todas as interfaces
// (Docker, VPNs, agregadores como o net-merge) ou os soquetes extras do STUN, ele
// às vezes escolhia um caminho morto e a conexão nunca fechava.
const configMediaMTX = `
logLevel: info
rtsp: false
rtmp: false
hls: false
srt: false
moq: false

webrtc: true
webrtcAddress: 127.0.0.1:%d
webrtcLocalUDPAddress: %s:%d
webrtcLocalTCPAddress: ""
webrtcIPsFromInterfaces: false
webrtcAdditionalHosts: ["%s"]
webrtcICEServers2:
  # Só anunciada aos clientes (o OBS lê dos cabeçalhos Link do WHIP).
  - url: turns:%s:%d?transport=tcp
    username: %s
    password: %s
    clientOnly: true

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

func iniciarMediaMTX(dns, segredoTURN string, ipRelay net.IP) (*exec.Cmd, error) {
	conf := filepath.Join(pastaDados, "mediamtx.yml")
	texto := fmt.Sprintf(configMediaMTX, portaWebRTC, ipRelay, portaMidia, ipRelay, dns, portaTURNPublica, usuarioTURN, segredoTURN, portaAPI)
	if err := os.WriteFile(conf, []byte(texto), 0o600); err != nil {
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

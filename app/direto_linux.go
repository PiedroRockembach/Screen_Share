package main

import (
	"errors"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/godbus/dbus/v5"
)

func (a *App) comandoGst(args ...string) (*exec.Cmd, error) {
	p, err := exec.LookPath("gst-launch-1.0")
	if err != nil {
		return nil, errors.New("instale o GStreamer: sudo pacman -S gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav gst-plugin-pipewire")
	}
	cmd := exec.Command(p, args...)
	prepararFilho(cmd)
	return cmd, nil
}

func liberarFirewallSRT(a *App) error {
	if _, err := exec.LookPath("ufw"); err != nil {
		return nil
	}
	log.Print(">>> Digite a sua senha na janela que abrir (libera a porta do teste no firewall, só dentro do Tailscale).")
	regra := fmt.Sprintf("if ufw status | grep -q 'Status: active'; then ufw allow in on tailscale0 to any port %d proto udp comment 'Sala de Tela teste'; fi", portaSRT)
	saida, err := exec.Command("pkexec", "sh", "-c", regra).CombinedOutput()
	if err != nil {
		return fmt.Errorf("%v %s", err, strings.TrimSpace(string(saida)))
	}
	return nil
}

// Portal ScreenCast: pede ao sistema (Wayland ou X11) a captura da tela e
// devolve o acesso ao PipeWire. O KDE/GNOME mostra a janela para escolher a tela.
type capturaPortal struct {
	conn *dbus.Conn
	fd   *os.File
	no   uint32
}

func pedirCaptura() (*capturaPortal, error) {
	conn, err := dbus.ConnectSessionBus()
	if err != nil {
		return nil, fmt.Errorf("conectar ao D-Bus: %w", err)
	}
	portal := conn.Object("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop")
	const iface = "org.freedesktop.portal.ScreenCast."
	remetente := strings.ReplaceAll(strings.TrimPrefix(conn.Names()[0], ":"), ".", "_")
	sinais := make(chan *dbus.Signal, 8)
	conn.Signal(sinais)
	contador := 0

	// Cada chamada responde depois, por um sinal Response no caminho do pedido.
	chamar := func(metodo string, opcoes map[string]dbus.Variant, args ...any) (map[string]dbus.Variant, error) {
		contador++
		token := fmt.Sprintf("sala%d", contador)
		caminho := dbus.ObjectPath(fmt.Sprintf("/org/freedesktop/portal/desktop/request/%s/%s", remetente, token))
		if err := conn.AddMatchSignal(dbus.WithMatchObjectPath(caminho), dbus.WithMatchInterface("org.freedesktop.portal.Request"), dbus.WithMatchMember("Response")); err != nil {
			return nil, err
		}
		opcoes["handle_token"] = dbus.MakeVariant(token)
		var pedido dbus.ObjectPath
		if err := portal.Call(iface+metodo, 0, append(args, opcoes)...).Store(&pedido); err != nil {
			return nil, fmt.Errorf("%s: %w", metodo, err)
		}
		limite := time.After(3 * time.Minute)
		for {
			select {
			case sinal := <-sinais:
				if sinal.Path != caminho || len(sinal.Body) < 2 {
					continue
				}
				if codigo, _ := sinal.Body[0].(uint32); codigo != 0 {
					return nil, errors.New("a captura da tela foi cancelada")
				}
				resultado, _ := sinal.Body[1].(map[string]dbus.Variant)
				return resultado, nil
			case <-limite:
				return nil, errors.New("o sistema não respondeu ao pedido de captura da tela")
			}
		}
	}
	falhar := func(err error) (*capturaPortal, error) {
		conn.Close()
		return nil, err
	}

	r, err := chamar("CreateSession", map[string]dbus.Variant{"session_handle_token": dbus.MakeVariant("salasessao")})
	if err != nil {
		return falhar(err)
	}
	textoSessao, _ := r["session_handle"].Value().(string)
	sessao := dbus.ObjectPath(textoSessao)
	if _, err := chamar("SelectSources", map[string]dbus.Variant{
		"types":       dbus.MakeVariant(uint32(1)), // monitor
		"multiple":    dbus.MakeVariant(false),
		"cursor_mode": dbus.MakeVariant(uint32(2)), // cursor na imagem
	}, sessao); err != nil {
		return falhar(err)
	}
	r, err = chamar("Start", map[string]dbus.Variant{}, sessao, "")
	if err != nil {
		return falhar(err)
	}
	var fluxos []struct {
		No    uint32
		Props map[string]dbus.Variant
	}
	if err := dbus.Store([]any{r["streams"].Value()}, &fluxos); err != nil || len(fluxos) == 0 {
		return falhar(errors.New("o sistema não devolveu nenhuma tela para capturar"))
	}
	var fd dbus.UnixFD
	if err := portal.Call(iface+"OpenPipeWireRemote", 0, sessao, map[string]dbus.Variant{}).Store(&fd); err != nil {
		return falhar(fmt.Errorf("OpenPipeWireRemote: %w", err))
	}
	return &capturaPortal{conn: conn, fd: os.NewFile(uintptr(fd), "pipewire"), no: fluxos[0].No}, nil
}

func encoderLinux() (nome, elemento string, largura, altura int) {
	tem := func(e string) bool { return exec.Command("gst-inspect-1.0", "--exists", e).Run() == nil }
	switch {
	case tem("nvh264enc"):
		return "NVIDIA", "nvh264enc bitrate=9000 rc-mode=cbr gop-size=60 bframes=0 zerolatency=true preset=p4 tune=ultra-low-latency ! video/x-h264,profile=high", 1920, 1080
	case tem("vah264enc"):
		return "VA-API", "vah264enc bitrate=9000 rate-control=cbr key-int-max=60 b-frames=0", 1920, 1080
	default:
		return "processador (720p)", "x264enc bitrate=4500 tune=zerolatency speed-preset=veryfast key-int-max=60 bframes=0", 1280, 720
	}
}

func pipelineLinux(fonte, uri string, comSom bool) ([]string, string) {
	nome, enc, w, h := encoderLinux()
	p := fonte + " ! videorate ! videoconvertscale n-threads=4 ! " +
		fmt.Sprintf("video/x-raw,format=NV12,width=%d,height=%d,framerate=60/1,pixel-aspect-ratio=1/1", w, h) +
		" ! queue max-size-buffers=2 leaky=downstream ! " + enc + finalEnvio(uri)
	if comSom {
		p += " pulsesrc device=" + sinkSala + ".monitor do-timestamp=true ! queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 ! queue max-size-time=6000000000 max-size-buffers=0 max-size-bytes=0 ! mux."
	}
	return strings.Fields(p), fmt.Sprintf("%dx%d 60fps, %s", w, h, nome)
}

func (a *App) enviarDireto(uri string, comSom bool) (*Transmissao, error) {
	captura, err := func() (*capturaPortal, error) {
		log.Print(">>> Escolha a tela na janela do sistema.")
		return pedirCaptura()
	}()
	if err != nil {
		return nil, err
	}
	t := novaTransmissao()
	aviso := "sem som"
	if comSom {
		if iniciarAudioSala(t.fim) {
			aviso = "com o som do PC, sem o Discord"
		} else {
			comSom = false
			aviso = "SEM som: não consegui criar a saída de áudio da Sala"
		}
	}
	fonte := fmt.Sprintf("pipewiresrc fd=3 path=%d do-timestamp=true keepalive-time=100", captura.no)
	args, descricao := pipelineLinux(fonte, uri, comSom)
	cmd, err := a.comandoGst(args...)
	if err != nil {
		t.cancelar()
		captura.conn.Close()
		return nil, err
	}
	cmd.ExtraFiles = []*os.File{captura.fd}
	cmd.Env = append(os.Environ(), "GST_DEBUG=2", "GST_DEBUG_NO_COLOR=1")
	if f, err := os.Create(filepath.Join(pastaDados, "teste-direto-envio.log")); err == nil {
		cmd.Stdout, cmd.Stderr = f, f
	}
	if err := cmd.Start(); err != nil {
		t.cancelar()
		captura.conn.Close()
		return nil, err
	}
	t.definir(descricao + ", " + aviso)
	go func() {
		defer close(t.parada)
		defer captura.conn.Close()
		saiu := make(chan error, 1)
		go func() { saiu <- cmd.Wait() }()
		select {
		case <-t.fim:
			cmd.Process.Kill()
			<-saiu
		case err := <-saiu:
			log.Printf("a transmissão fechou (%v); veja teste-direto-envio.log", err)
			t.cancelar()
		}
	}()
	return t, nil
}

func diagnosticoDireto(a *App) {
	for _, e := range []string{"pipewiresrc", "srtsink", "srtsrc", "mpegtsmux", "tsdemux", "playbin", "nvh264enc", "vah264enc", "x264enc", "opusenc", "pulsesrc"} {
		estado := "AUSENTE"
		if exec.Command("gst-inspect-1.0", "--exists", e).Run() == nil {
			estado = "ok"
		}
		log.Printf("  %-14s %s", e, estado)
	}
}

package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"golang.org/x/sys/windows/registry"
)

// Transmissão no Windows: GStreamer captura a tela (D3D11), codifica no
// encoder de hardware e envia via WHIP. O som do PC é capturado SEM a árvore
// de processos do Discord (wasapi2src loopback-mode=exclude-process-tree).

const versaoGst = "1.28.8"

var candidatosEncoder = []struct {
	EncoderWin
	props []string // completa e mínima de reserva
}{
	{EncoderWin{Nome: "NVIDIA", El: "nvd3d11h264enc", D3D: true, W: 1920, H: 1080},
		[]string{"bitrate=9000 gop-size=60 bframes=0 rc-mode=cbr zero-reorder-delay=true", "bitrate=9000"}},
	{EncoderWin{Nome: "AMD", El: "amfh264enc", D3D: true, W: 1920, H: 1080},
		[]string{"bitrate=9000 gop-size=60 rate-control=cbr usage=low-latency", "bitrate=9000"}},
	{EncoderWin{Nome: "Intel", El: "qsvh264enc", D3D: true, W: 1920, H: 1080},
		[]string{"bitrate=9000 gop-size=60 b-frames=0 rate-control=cbr", "bitrate=9000"}},
	{EncoderWin{Nome: "NVIDIA (CUDA)", El: "nvh264enc", W: 1920, H: 1080},
		[]string{"bitrate=9000 gop-size=60 bframes=0 rc-mode=cbr", "bitrate=9000"}},
	{EncoderWin{Nome: "Windows (Media Foundation)", El: "mfh264enc", W: 1920, H: 1080},
		[]string{"bitrate=9000 gop-size=60 bframes=0 rc-mode=cbr low-latency=true", "bitrate=9000"}},
	{EncoderWin{Nome: "Software (x264, 720p)", El: "x264enc", W: 1280, H: 720},
		[]string{"bitrate=4500 tune=zerolatency speed-preset=veryfast key-int-max=60 bframes=0"}},
}

func pastaGst() string {
	var raizes []string
	if k, err := registry.OpenKey(registry.LOCAL_MACHINE, `SYSTEM\CurrentControlSet\Control\Session Manager\Environment`, registry.QUERY_VALUE); err == nil {
		if v, _, err := k.GetStringValue("GSTREAMER_1_0_ROOT_MSVC_X86_64"); err == nil {
			raizes = append(raizes, v)
		}
		k.Close()
	}
	raizes = append(raizes, filepath.Join(os.Getenv("ProgramFiles"), `gstreamer\1.0\msvc_x86_64`), `C:\gstreamer\1.0\msvc_x86_64`)
	for _, r := range raizes {
		bin := filepath.Join(r, "bin")
		if _, err := os.Stat(filepath.Join(bin, "gst-launch-1.0.exe")); err == nil {
			return bin
		}
	}
	return ""
}

func gst(bin, prog string, args ...string) *exec.Cmd {
	cmd := exec.Command(filepath.Join(bin, prog+".exe"), args...)
	cmd.Dir = bin
	prepararFilho(cmd)
	return cmd
}

func rodarComLimite(cmd *exec.Cmd, limite time.Duration) bool {
	if err := cmd.Start(); err != nil {
		return false
	}
	vincularFilho(cmd)
	fim := make(chan error, 1)
	go func() { fim <- cmd.Wait() }()
	select {
	case err := <-fim:
		return err == nil
	case <-time.After(limite):
		cmd.Process.Kill()
		return false
	}
}

func temElemento(bin, el string) bool {
	return rodarComLimite(gst(bin, "gst-inspect-1.0", "--exists", el), 2*time.Minute)
}

func (a *App) garantirGStreamer() (string, error) {
	if bin := pastaGst(); bin != "" {
		return bin, nil
	}
	arq := fmt.Sprintf("gstreamer-1.0-msvc-x86_64-%s.exe", versaoGst)
	url := fmt.Sprintf("https://gstreamer.freedesktop.org/data/pkg/windows/%s/msvc/%s", versaoGst, arq)
	exe := filepath.Join(pastaDados, "downloads", arq)
	a.registrar("baixando o GStreamer (~540 MB, só na primeira vez)…")
	if err := a.baixarVerificado(url, url+".sha256sum", exe); err != nil {
		return "", err
	}
	a.definirAcao("Clique em \"Sim\" no aviso do Windows para instalar o GStreamer", "")
	err := comoAdministrador(exe, "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /ALLUSERS /TYPE=runtime")
	a.definirAcao("", "")
	if err != nil {
		return "", err
	}
	os.Remove(exe)
	bin := pastaGst()
	if bin == "" {
		return "", errors.New("o GStreamer foi instalado mas não foi encontrado")
	}
	a.registrar("preparando o GStreamer (a primeira vez demora)…")
	temElemento(bin, "whipclientsink") // monta o cache de plugins
	return bin, nil
}

func (a *App) detectarEncoder(bin string) (*EncoderWin, error) {
	saida, _ := gst(bin, "gst-launch-1.0", "--version").Output()
	versao := strings.TrimSpace(string(saida))
	if c := a.config(); c.Encoder != nil && c.GstVersao == versao {
		return c.Encoder, nil
	}
	for _, el := range []string{"d3d11screencapturesrc", "whipclientsink"} {
		if !temElemento(bin, el) {
			return nil, fmt.Errorf("falta o componente %s do GStreamer", el)
		}
	}
	for _, c := range candidatosEncoder {
		if !temElemento(bin, c.El) {
			continue
		}
		for _, props := range c.props {
			a.registrar("testando o codificador %s…", c.Nome)
			conv := ""
			if !c.D3D {
				conv = " ! d3d11download ! videoconvert ! video/x-raw,format=NV12"
			}
			teste := fmt.Sprintf("-q d3d11testsrc num-buffers=30 ! video/x-raw(memory:D3D11Memory),width=%d,height=%d,framerate=60/1%s ! %s %s ! h264parse ! fakesink",
				c.W, c.H, conv, c.El, props)
			if rodarComLimite(gst(bin, "gst-launch-1.0", strings.Fields(teste)...), time.Minute) {
				e := c.EncoderWin
				e.Props = props
				a.alterarConfig(func(cfg *Config) { cfg.Encoder, cfg.GstVersao = &e, versao })
				a.registrar("codificador escolhido: %s", e.Nome)
				return &e, nil
			}
		}
	}
	return nil, errors.New("nenhum codificador de vídeo funcionou neste PC")
}

// raizesDiscord: os Discord.exe cujo pai não é outro Discord (excluir essa
// árvore tira todo o som do Discord da captura).
func raizesDiscord() []uint32 {
	procs, err := listarProcessos()
	if err != nil {
		return nil
	}
	ehDiscord := map[uint32]bool{}
	for _, p := range procs {
		if reDiscord.MatchString(p.Nome) {
			ehDiscord[p.PID] = true
		}
	}
	var raizes []uint32
	for _, p := range procs {
		if ehDiscord[p.PID] && !ehDiscord[p.Pai] {
			raizes = append(raizes, p.PID)
		}
	}
	return raizes
}

func montarPipeline(e *EncoderWin, whip string, pidDiscord uint32) []string {
	p := fmt.Sprintf("-e whipclientsink name=ws congestion-control=disabled signaller::whip-endpoint=%s", whip) +
		" d3d11screencapturesrc show-cursor=true ! video/x-raw(memory:D3D11Memory),framerate=60/1" +
		fmt.Sprintf(" ! d3d11convert ! video/x-raw(memory:D3D11Memory),width=%d,height=%d,pixel-aspect-ratio=1/1", e.W, e.H)
	if !e.D3D {
		p += " ! d3d11download ! videoconvert ! video/x-raw,format=NV12"
	}
	p += fmt.Sprintf(" ! queue max-size-buffers=2 leaky=downstream ! %s %s ! h264parse config-interval=-1 ! ws.", e.El, e.Props)
	if pidDiscord != 0 {
		p += fmt.Sprintf(" wasapi2src loopback=true low-latency=true loopback-mode=exclude-process-tree loopback-target-pid=%d", pidDiscord) +
			" ! queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 ! ws."
	}
	return strings.Fields(p)
}

func (a *App) transmitirSO(whip, nome string, comSom bool) (*Transmissao, error) {
	bin, err := a.garantirGStreamer()
	if err != nil {
		return nil, err
	}
	enc, err := a.detectarEncoder(bin)
	if err != nil {
		return nil, err
	}
	audioPossivel := comSom && versaoWindows() >= 20348
	logPath := filepath.Join(pastaDados, "transmissao.log")

	t := novaTransmissao()
	var proc *exec.Cmd
	var pidExcluido uint32

	iniciar := func() error {
		raizes := raizesDiscord()
		pidExcluido = 0
		aviso := "sem som (desmarcado)"
		switch {
		case !comSom:
		case !audioPossivel:
			aviso = "sem som (precisa do Windows 11)"
		case len(raizes) == 1:
			pidExcluido = raizes[0]
			aviso = "com o som do PC, sem o Discord"
		case len(raizes) == 0:
			aviso = "SEM som: o app do Discord não está aberto (abra e o som liga sozinho)"
		default:
			aviso = "SEM som: há mais de um Discord aberto"
		}
		proc = gst(bin, "gst-launch-1.0", montarPipeline(enc, whip, pidExcluido)...)
		proc.Env = append(os.Environ(), "GST_DEBUG=2", "GST_DEBUG_NO_COLOR=1")
		if f, err := os.Create(logPath); err == nil {
			proc.Stdout, proc.Stderr = f, f
		}
		if err := proc.Start(); err != nil {
			return err
		}
		vincularFilho(proc)
		t.definir(fmt.Sprintf("AO VIVO como \"%s\" (%dx%d 60fps, %s), %s", nome, enc.W, enc.H, enc.Nome, aviso))
		return nil
	}
	if err := iniciar(); err != nil {
		return nil, err
	}

	go func() {
		defer close(t.parada)
		esperar := func(c *exec.Cmd) chan struct{} {
			ch := make(chan struct{})
			go func() { c.Wait(); close(ch) }()
			return ch
		}
		saiu := esperar(proc)
		falhas := 0
		for {
			select {
			case <-t.fim:
				proc.Process.Kill()
				<-saiu
				return
			case <-saiu:
				falhas++
				if falhas > 3 {
					t.definir("a transmissão caiu várias vezes (veja transmissao.log)")
					return
				}
				t.definir(fmt.Sprintf("a transmissão caiu; tentando de novo (%d/3)…", falhas))
				time.Sleep(2 * time.Second)
				if iniciar() != nil {
					return
				}
				saiu = esperar(proc)
			case <-time.After(time.Second):
				// O PID excluído precisa ser o do Discord atual: se ele abriu, fechou ou
				// reiniciou, para NA HORA e recomeça (senão a voz poderia vazar).
				if !audioPossivel {
					continue
				}
				raizes := raizesDiscord()
				mudou := len(raizes) == 1 && raizes[0] != pidExcluido || len(raizes) != 1 && pidExcluido != 0
				if mudou {
					proc.Process.Kill()
					<-saiu
					t.definir("o Discord abriu ou fechou: reiniciando a transmissão…")
					if iniciar() != nil {
						return
					}
					saiu = esperar(proc)
				}
			}
		}
	}()
	return t, nil
}

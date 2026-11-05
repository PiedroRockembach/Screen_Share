package main

import (
	"fmt"
	"log"
	"os/exec"
	"strings"
)

func (a *App) comandoGst(args ...string) (*exec.Cmd, error) {
	bin, err := a.garantirGStreamer()
	if err != nil {
		return nil, err
	}
	return gst(bin, "gst-launch-1.0", args...), nil
}

func liberarFirewallSRT(a *App) error {
	log.Print(">>> Clique em \"Sim\" no aviso do Windows (libera a porta do teste no firewall).")
	return comoAdministrador("netsh", fmt.Sprintf(`advfirewall firewall add rule name="Sala de Tela teste" dir=in action=allow protocol=UDP localport=%d`, portaSRT))
}

func pipelineWindows(e *EncoderWin, uri string, pidDiscord uint32) []string {
	p := "-e d3d11screencapturesrc show-cursor=true ! video/x-raw(memory:D3D11Memory),framerate=60/1" +
		fmt.Sprintf(" ! d3d11convert ! video/x-raw(memory:D3D11Memory),width=%d,height=%d,pixel-aspect-ratio=1/1", e.W, e.H)
	if !e.D3D {
		p += " ! d3d11download ! videoconvert ! video/x-raw,format=NV12"
	}
	p += fmt.Sprintf(" ! queue max-size-buffers=2 leaky=downstream ! %s %s", e.El, e.Props) + finalEnvio(uri)
	if pidDiscord != 0 {
		p += fmt.Sprintf(" wasapi2src loopback=true low-latency=true loopback-mode=exclude-process-tree loopback-target-pid=%d", pidDiscord) +
			" ! queue ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 ! queue max-size-time=6000000000 max-size-buffers=0 max-size-bytes=0 ! mux."
	}
	return strings.Fields(p)
}

func (a *App) enviarDireto(uri string, comSom bool) (*Transmissao, error) {
	bin, err := a.garantirGStreamer()
	if err != nil {
		return nil, err
	}
	enc, err := a.detectarEncoder(bin, "srtsink", "mpegtsmux")
	if err != nil {
		return nil, err
	}
	return enviarWindows(bin, comSom, "teste-direto-envio.log",
		func(pid uint32) []string { return pipelineWindows(enc, uri, pid) },
		func(t *Transmissao, aviso string) {
			t.definir(fmt.Sprintf("%dx%d 60fps, %s, %s", enc.W, enc.H, enc.Nome, aviso))
		})
}

func diagnosticoDireto(a *App) {
	log.Printf("Windows build %d (o som sem Discord exige 20348 ou mais)", versaoWindows())
	bin, err := a.garantirGStreamer()
	if err != nil {
		log.Printf("GStreamer: %v", err)
		return
	}
	for _, e := range []string{"d3d11screencapturesrc", "d3d11testsrc", "srtsink", "srtsrc", "mpegtsmux", "tsdemux", "playbin", "wasapi2src", "opusenc", "opusdec", "d3d11h264dec", "avdec_h264", "nvd3d11h264enc", "amfh264enc", "qsvh264enc", "mfh264enc", "x264enc"} {
		estado := "AUSENTE"
		if temElemento(bin, e) {
			estado = "ok"
		}
		log.Printf("  %-22s %s", e, estado)
	}
	saida, _ := gst(bin, "gst-inspect-1.0", "wasapi2src").Output()
	log.Printf("  exclusão do Discord (loopback-mode): %v", strings.Contains(string(saida), "loopback-mode"))
	log.Printf("  processos raiz do Discord: %v", raizesDiscord())
}

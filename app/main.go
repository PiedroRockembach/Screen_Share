// Sala de Tela: compartilhamento de tela 1080p60 entre amigos, sem o Discord.
//
// Um único programa para Linux e Windows. Ao abrir, ele mostra um painel no
// navegador (só neste PC) para:
//   - criar uma sala: sobe o MediaMTX e publica a sala na internet pelo
//     Tailscale Funnel, com um link de convite único para todos;
//   - transmitir a tela (com o som do PC, mas sem o Discord);
//   - assistir (a página da sala abre em qualquer navegador).
package main

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
)

const (
	portaPainel    = 47800 // painel de controle, só 127.0.0.1
	portaSala      = 47801 // página e WHIP/WHEP da sala, publicada pelo Funnel
	versaoMediaMTX = "v1.21.2"
)

var pastaDados string

func main() {
	pastaDados = diretorioDados()
	if err := os.MkdirAll(pastaDados, 0o755); err != nil {
		log.Fatal(err)
	}
	arqLog, err := os.OpenFile(filepath.Join(pastaDados, "sala-de-tela.log"), os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o644)
	if err == nil {
		log.SetOutput(io.MultiWriter(os.Stderr, arqLog))
	}

	app := novoApp()
	urlPainel := fmt.Sprintf("http://127.0.0.1:%d/?k=%s", portaPainel, app.cfg.ChavePainel)

	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", portaPainel))
	if err != nil {
		// Já está aberto: só mostra o painel de novo.
		abrirNavegador(urlPainel)
		return
	}
	lnSala, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", portaSala))
	if err != nil {
		log.Fatalf("porta %d ocupada: %v", portaSala, err)
	}

	go func() {
		sinais := make(chan os.Signal, 1)
		signal.Notify(sinais, os.Interrupt, syscall.SIGTERM)
		<-sinais
		app.encerrar()
	}()

	go http.Serve(lnSala, app.rotasSala())
	log.Printf("painel em %s", urlPainel)
	abrirNavegador(urlPainel)
	if err := http.Serve(ln, app.rotasPainel()); err != nil {
		log.Fatal(err)
	}
}

func aleatorio(bytes int) string {
	b := make([]byte, bytes)
	if _, err := rand.Read(b); err != nil {
		panic(err)
	}
	return hex.EncodeToString(b)
}

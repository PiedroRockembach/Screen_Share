package main

import (
	"encoding/json"
	"regexp"
	"strings"
	"sync"
	"time"
)

// Transmissao é a transmissão da tela DESTE PC (para a própria sala ou a de um amigo).
type Transmissao struct {
	mu      sync.Mutex
	ativa   bool
	status  string
	fim     chan struct{} // fechado para pedir que pare
	parada  chan struct{} // fechado quando tudo terminou
	geracao int           // cada (re)início invalida o acompanhamento anterior
}

func novaTransmissao() *Transmissao {
	return &Transmissao{ativa: true, fim: make(chan struct{}), parada: make(chan struct{})}
}

func (t *Transmissao) estado() (bool, string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.ativa, t.status
}

func (t *Transmissao) definir(status string) {
	t.mu.Lock()
	t.status = status
	t.mu.Unlock()
}

// acompanhar só diz AO VIVO quando a sala confirma que o vídeo chegou nela.
// Abrir o programa de captura não garante isso: a conexão pode nunca fechar.
func (t *Transmissao) acompanhar(whip, nome, detalhes string) {
	t.mu.Lock()
	t.geracao++
	g := t.geracao
	t.status = "Conectando à sala… " + detalhes
	t.mu.Unlock()
	definir := func(s string) {
		t.mu.Lock()
		if t.geracao == g {
			t.status = s
		}
		t.mu.Unlock()
	}
	api := strings.TrimSuffix(whip, nome+"/whip") + "api/ao-vivo"
	go func() {
		inicio := time.Now()
		for {
			select {
			case <-t.fim:
				return
			case <-time.After(2 * time.Second):
			}
			if corpo, err := lerURL(api); err == nil {
				var lista []transmissao
				if json.Unmarshal([]byte(corpo), &lista) == nil {
					for _, item := range lista {
						if item.Nome == nome {
							definir("AO VIVO: " + detalhes)
							return
						}
					}
				}
			}
			if time.Since(inicio) > 30*time.Second {
				definir("O vídeo ainda NÃO chegou na sala: ninguém consegue ver. Continuo tentando. " + detalhes)
			}
		}
	}()
}

// cancelar pede para parar sem esperar.
func (t *Transmissao) cancelar() {
	t.mu.Lock()
	if t.ativa {
		t.ativa = false
		close(t.fim)
	}
	t.mu.Unlock()
}

// parar pede para parar e espera tudo terminar.
func (t *Transmissao) parar() {
	t.cancelar()
	<-t.parada
}

func (a *App) iniciarTransmissao(base, nome string, comSom bool) error {
	a.pararTransmissao()
	t, err := a.transmitirSO(base+nome+"/whip", nome, comSom)
	if err != nil {
		return err
	}
	a.mu.Lock()
	a.trans = t
	a.mu.Unlock()
	go func() {
		<-t.parada
		t.mu.Lock()
		t.ativa = false
		t.mu.Unlock()
		a.registrar("transmissão encerrada")
	}()
	return nil
}

func (a *App) pararTransmissao() {
	a.mu.Lock()
	t := a.trans
	a.mu.Unlock()
	if t != nil {
		t.parar()
	}
}

// EncoderWin é o codificador H.264 que funcionou neste PC Windows (fica salvo na config).
type EncoderWin struct {
	Nome  string `json:"nome"`
	El    string `json:"el"`
	Props string `json:"props"`
	D3D   bool   `json:"d3d"` // aceita textura da GPU direto
	W     int    `json:"w"`
	H     int    `json:"h"`
}

// Nomes de programa do Discord e clientes alternativos (o som deles nunca é transmitido).
var reDiscord = regexp.MustCompile(`(?i)discord|vesktop|webcord|legcord|armcord`)

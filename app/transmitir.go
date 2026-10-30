package main

import (
	"regexp"
	"sync"
)

// Transmissao é a transmissão da tela DESTE PC (para a própria sala ou a de um amigo).
type Transmissao struct {
	mu     sync.Mutex
	ativa  bool
	status string
	fim    chan struct{} // fechado para pedir que pare
	parada chan struct{} // fechado quando tudo terminou
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

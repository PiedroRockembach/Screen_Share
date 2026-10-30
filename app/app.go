package main

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"
)

// Config fica salva em pastaDados/config.json.
type Config struct {
	ChavePainel   string      `json:"chavePainel"`   // protege o painel local contra outros sites
	Token         string      `json:"token"`         // parte secreta do link da sala
	Nome          string      `json:"nome"`          // nome de quem transmite
	ComSom        bool        `json:"comSom"`        // transmitir o som do PC (sem o Discord)
	LinkConvidado string      `json:"linkConvidado"` // último link de sala usado para transmitir
	Encoder       *EncoderWin `json:"encoder,omitempty"`
	GstVersao     string      `json:"gstVersao,omitempty"`

	FirewallLiberado bool `json:"firewallLiberado,omitempty"` // Linux: regra do ufw já criada
}

type App struct {
	mu    sync.Mutex
	cfg   Config
	sala  *Sala
	trans *Transmissao

	// Para o painel: o que está acontecendo agora e um histórico curto.
	ocupado   string
	urlAcao   string // link que a pessoa precisa abrir (login, ativar Funnel)
	textoAcao string
	erro      string
	historico []string
}

func novoApp() *App {
	a := &App{}
	a.cfg = Config{ComSom: true}
	if b, err := os.ReadFile(a.arqConfig()); err == nil {
		_ = json.Unmarshal(b, &a.cfg)
	}
	if a.cfg.ChavePainel == "" {
		a.cfg.ChavePainel = aleatorio(16)
	}
	if a.cfg.Token == "" {
		a.cfg.Token = aleatorio(16)
	}
	if a.cfg.Nome == "" {
		a.cfg.Nome = nomeSeguro(usuarioDoSistema())
	}
	a.alterarConfig(func(*Config) {})
	return a
}

func (a *App) arqConfig() string { return filepath.Join(pastaDados, "config.json") }

// alterarConfig muda a config com a trava e grava no disco.
func (a *App) alterarConfig(f func(*Config)) {
	a.mu.Lock()
	f(&a.cfg)
	b, _ := json.MarshalIndent(a.cfg, "", "  ")
	a.mu.Unlock()
	if err := os.WriteFile(a.arqConfig(), b, 0o600); err != nil {
		log.Printf("salvar config: %v", err)
	}
}

func (a *App) config() Config {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.cfg
}

// registrar mostra uma linha no painel e no log.
func (a *App) registrar(formato string, args ...any) {
	linha := fmt.Sprintf(formato, args...)
	log.Print(linha)
	a.mu.Lock()
	defer a.mu.Unlock()
	a.historico = append(a.historico, time.Now().Format("15:04:05")+"  "+linha)
	if len(a.historico) > 60 {
		a.historico = a.historico[len(a.historico)-60:]
	}
}

func (a *App) definirAcao(texto, url string) {
	a.mu.Lock()
	a.textoAcao, a.urlAcao = texto, url
	a.mu.Unlock()
	if url != "" {
		abrirNavegador(url)
	}
}

// tarefa roda uma operação demorada em segundo plano, uma de cada vez.
func (a *App) tarefa(descricao string, f func() error) bool {
	a.mu.Lock()
	if a.ocupado != "" {
		a.mu.Unlock()
		return false
	}
	a.ocupado, a.erro = descricao, ""
	a.mu.Unlock()
	go func() {
		err := f()
		a.mu.Lock()
		a.ocupado, a.textoAcao, a.urlAcao = "", "", ""
		if err != nil {
			a.erro = err.Error()
		}
		a.mu.Unlock()
		if err != nil {
			a.registrar("ERRO: %v", err)
		}
	}()
	return true
}

func (a *App) encerrar() {
	a.registrar("encerrando…")
	if t := a.transAtual(); t != nil {
		t.parar()
	}
	if s := a.salaAtual(); s != nil {
		s.parar()
	}
	os.Exit(0)
}

func (a *App) salaAtual() *Sala {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.sala
}

func (a *App) transAtual() *Transmissao {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.trans
}

var naoPermitido = regexp.MustCompile(`[^a-z0-9_-]+`)

// nomeSeguro vira o caminho da transmissão no MediaMTX (/joao, /maria...).
func nomeSeguro(s string) string {
	s = strings.ToLower(s)
	s = strings.NewReplacer("á", "a", "à", "a", "â", "a", "ã", "a", "é", "e", "ê", "e", "í", "i",
		"ó", "o", "ô", "o", "õ", "o", "ú", "u", "ü", "u", "ç", "c").Replace(s)
	s = strings.Trim(naoPermitido.ReplaceAllString(s, "-"), "-")
	if len(s) > 32 {
		s = s[:32]
	}
	return s
}

func usuarioDoSistema() string {
	for _, v := range []string{"USER", "USERNAME"} {
		if u := os.Getenv(v); u != "" {
			return u
		}
	}
	return "eu"
}

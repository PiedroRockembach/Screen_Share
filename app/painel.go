package main

import (
	"crypto/subtle"
	"encoding/json"
	"fmt"
	"io/fs"
	"net/http"
	"net/url"
	"runtime"
	"strings"
)

type estadoPainel struct {
	SO           string   `json:"so"`
	Ocupado      string   `json:"ocupado"`
	Erro         string   `json:"erro"`
	TextoAcao    string   `json:"textoAcao"`
	URLAcao      string   `json:"urlAcao"`
	SalaAtiva    bool     `json:"salaAtiva"`
	LinkSala     string   `json:"linkSala"`
	LinkLocal    string   `json:"linkLocal"`
	Nome         string   `json:"nome"`
	ComSom       bool     `json:"comSom"`
	LinkUltimo   string   `json:"linkUltimo"`
	Transmitindo bool     `json:"transmitindo"`
	StatusTrans  string   `json:"statusTrans"`
	Historico    []string `json:"historico"`
}

func (a *App) estado() estadoPainel {
	link := a.linkSala()
	var transmitindo bool
	var statusTrans string
	if t := a.transAtual(); t != nil {
		transmitindo, statusTrans = t.estado()
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	e := estadoPainel{
		SO: runtime.GOOS, Ocupado: a.ocupado, Erro: a.erro, TextoAcao: a.textoAcao, URLAcao: a.urlAcao,
		SalaAtiva: a.sala != nil, LinkSala: link,
		Nome: a.cfg.Nome, ComSom: a.cfg.ComSom, LinkUltimo: a.cfg.LinkConvidado,
		Historico: append([]string(nil), a.historico...),
	}
	if a.sala != nil {
		e.LinkLocal = fmt.Sprintf("http://127.0.0.1:%d/s/%s/", portaSala, a.cfg.Token)
	}
	e.Transmitindo, e.StatusTrans = transmitindo, statusTrans
	return e
}

func (a *App) rotasPainel() http.Handler {
	paginas, _ := fs.Sub(arquivosWeb, "web/painel")
	mux := http.NewServeMux()
	mux.Handle("GET /", http.FileServer(http.FS(paginas)))

	api := func(padrao string, f func(r *http.Request) (any, error)) {
		mux.HandleFunc(padrao, func(w http.ResponseWriter, r *http.Request) {
			// Só aceita o próprio painel: cabeçalho com a chave (outros sites não conseguem enviar).
			chave := r.Header.Get("X-Chave")
			if subtle.ConstantTimeCompare([]byte(chave), []byte(a.config().ChavePainel)) != 1 {
				http.Error(w, "proibido", http.StatusForbidden)
				return
			}
			r.Body = http.MaxBytesReader(w, r.Body, 64<<10)
			res, err := f(r)
			w.Header().Set("Content-Type", "application/json")
			if err != nil {
				w.WriteHeader(http.StatusBadRequest)
				res = map[string]string{"erro": err.Error()}
			}
			json.NewEncoder(w).Encode(res)
		})
	}

	api("GET /api/estado", func(*http.Request) (any, error) { return a.estado(), nil })

	api("POST /api/sala/criar", func(*http.Request) (any, error) {
		if !a.tarefa("Criando a sala", a.criarSala) {
			return nil, fmt.Errorf("espere a tarefa atual terminar")
		}
		return "ok", nil
	})
	api("POST /api/sala/fechar", func(*http.Request) (any, error) {
		a.fecharSala()
		return "ok", nil
	})
	api("POST /api/sala/novo-link", func(*http.Request) (any, error) {
		a.alterarConfig(func(c *Config) { c.Token = aleatorio(16) })
		a.registrar("link novo criado; o link antigo parou de funcionar")
		return "ok", nil
	})

	api("POST /api/transmitir", func(r *http.Request) (any, error) {
		var p struct {
			Link   string `json:"link"`
			Nome   string `json:"nome"`
			ComSom bool   `json:"comSom"`
		}
		if err := json.NewDecoder(r.Body).Decode(&p); err != nil {
			return nil, err
		}
		nome := nomeSeguro(p.Nome)
		if nome == "" {
			return nil, fmt.Errorf("escreva um nome (letras e números)")
		}
		base, err := a.baseTransmissao(p.Link)
		if err != nil {
			return nil, err
		}
		a.alterarConfig(func(c *Config) {
			c.Nome, c.ComSom = nome, p.ComSom
			if p.Link != "" {
				c.LinkConvidado = p.Link
			}
		})
		if !a.tarefa("Preparando a transmissão", func() error { return a.iniciarTransmissao(base, nome, p.ComSom) }) {
			return nil, fmt.Errorf("espere a tarefa atual terminar")
		}
		return "ok", nil
	})
	api("POST /api/transmitir/parar", func(*http.Request) (any, error) {
		a.pararTransmissao()
		return "ok", nil
	})

	api("POST /api/encerrar", func(*http.Request) (any, error) {
		go a.encerrar()
		return "ok", nil
	})
	return mux
}

// baseTransmissao devolve a URL base da sala (…/s/<token>/) onde transmitir.
// Sem link: a sala deste PC, pelo caminho local.
func (a *App) baseTransmissao(link string) (string, error) {
	link = strings.TrimSpace(link)
	if link == "" {
		if a.salaAtual() == nil {
			return "", fmt.Errorf("cole o link da sala (ou crie a sua sala primeiro)")
		}
		return a.linkLocal(), nil
	}
	u, err := url.Parse(link)
	if err != nil || u.Scheme != "https" || !strings.HasPrefix(u.Path, "/s/") {
		return "", fmt.Errorf("esse link não parece de uma Sala de Tela (https://…/s/…)")
	}
	partes := strings.Split(strings.Trim(u.Path, "/"), "/")
	if len(partes) < 2 {
		return "", fmt.Errorf("link incompleto: copie o link inteiro")
	}
	// A própria sala: usa o caminho local, sem dar a volta pela internet.
	if s := a.salaAtual(); s != nil && u.Host == s.dns {
		return a.linkLocal(), nil
	}
	return fmt.Sprintf("https://%s/s/%s/", u.Host, partes[1]), nil
}

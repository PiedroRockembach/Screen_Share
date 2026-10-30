package main

import (
	"crypto/subtle"
	"embed"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"os/exec"
	"regexp"
	"sort"
	"strings"
	"time"
)

//go:embed web/sala web/painel
var arquivosWeb embed.FS

// Sala é a sala que ESTE PC hospeda: MediaMTX + Funnel do Tailscale.
type Sala struct {
	dns      string
	mediamtx *exec.Cmd
	funnel   *exec.Cmd
}

func (s *Sala) parar() {
	for _, c := range []*exec.Cmd{s.funnel, s.mediamtx} {
		if c != nil && c.Process != nil {
			c.Process.Kill()
		}
	}
}

func (a *App) linkSala() string {
	s := a.salaAtual()
	if s == nil {
		return ""
	}
	return fmt.Sprintf("https://%s/s/%s/", s.dns, a.config().Token)
}

// linkLocal é o mesmo endereço, mas sem passar pela internet (para quem hospeda).
func (a *App) linkLocal() string {
	return fmt.Sprintf("http://127.0.0.1:%d/s/%s/", portaSala, a.config().Token)
}

func (a *App) criarSala() error {
	if a.salaAtual() != nil {
		return nil
	}
	// SALA_DEV=1: testes locais, sem Tailscale/Funnel (a sala só abre em 127.0.0.1).
	dev := os.Getenv("SALA_DEV") != ""
	dns := "localhost"
	if !dev {
		var err error
		if dns, err = a.garantirTailscale(); err != nil {
			return err
		}
	}
	if err := a.garantirMediaMTX(); err != nil {
		return err
	}
	mtx, err := iniciarMediaMTX()
	if err != nil {
		return err
	}
	// Espera a API do MediaMTX responder.
	for i := 0; ; i++ {
		if _, err := transmissoesAoVivo(); err == nil {
			break
		}
		if i == 20 {
			mtx.Process.Kill()
			return errors.New("o MediaMTX não iniciou (veja mediamtx.log na pasta de dados)")
		}
		time.Sleep(500 * time.Millisecond)
	}
	s := &Sala{dns: dns, mediamtx: mtx}
	if !dev {
		a.registrar("publicando a sala na internet (Tailscale Funnel)…")
		if s.funnel, err = a.iniciarFunnel(dns, portaSala); err != nil {
			mtx.Process.Kill()
			return err
		}
	}
	a.mu.Lock()
	a.sala = s
	a.mu.Unlock()
	a.registrar("sala aberta: %s", a.linkSala())

	// Se o MediaMTX ou o Funnel caírem, a sala fecha (e avisa).
	for _, c := range []*exec.Cmd{s.mediamtx, s.funnel} {
		if c == nil {
			continue
		}
		go func(c *exec.Cmd) {
			c.Wait()
			a.mu.Lock()
			ativa := a.sala == s
			if ativa {
				a.sala = nil
				a.erro = "a sala caiu (" + c.Path + " fechou); clique em Criar sala de novo"
			}
			a.mu.Unlock()
			if ativa {
				s.parar()
				a.registrar("a sala caiu: %s fechou", c.Path)
			}
		}(c)
	}
	return nil
}

func (a *App) fecharSala() {
	a.mu.Lock()
	s := a.sala
	a.sala = nil
	a.mu.Unlock()
	if s != nil {
		s.parar()
		a.registrar("sala fechada")
	}
}

// ---------------------------------------------------------------- servidor da sala

type transmissao struct {
	Nome         string   `json:"nome"`
	Faixas       []string `json:"faixas"`
	Espectadores int      `json:"espectadores"`
}

func transmissoesAoVivo() ([]transmissao, error) {
	cli := http.Client{Timeout: 3 * time.Second}
	resp, err := cli.Get(fmt.Sprintf("http://127.0.0.1:%d/v3/paths/list", portaAPI))
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var dados struct {
		Items []struct {
			Name    string
			Ready   bool
			Tracks  []string
			Readers []json.RawMessage
		}
	}
	if err := json.NewDecoder(resp.Body).Decode(&dados); err != nil {
		return nil, err
	}
	lista := []transmissao{}
	for _, it := range dados.Items {
		if it.Ready {
			lista = append(lista, transmissao{Nome: it.Name, Faixas: it.Tracks, Espectadores: len(it.Readers)})
		}
	}
	sort.Slice(lista, func(i, j int) bool { return lista[i].Nome < lista[j].Nome })
	return lista, nil
}

var reSinalizacao = regexp.MustCompile(`^([a-z0-9_-]{1,32})/(whip|whep)(/[0-9a-f-]{1,64})?$`)

// rotasSala atende https://<dns>/s/<token>/... (vindo do Funnel) e o acesso local.
// Sem o código certo no link, tudo responde 404.
func (a *App) rotasSala() http.Handler {
	paginas, _ := fs.Sub(arquivosWeb, "web/sala")
	estaticos := http.FileServer(http.FS(paginas))
	mtx, _ := url.Parse(fmt.Sprintf("http://127.0.0.1:%d", portaWebRTC))

	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Referrer-Policy", "no-referrer") // não vaza o código do link
		resto, ok := strings.CutPrefix(r.URL.Path, "/s/")
		if !ok || a.salaAtual() == nil {
			http.NotFound(w, r)
			return
		}
		token, caminho, _ := strings.Cut(resto, "/")
		if subtle.ConstantTimeCompare([]byte(token), []byte(a.config().Token)) != 1 {
			http.NotFound(w, r)
			return
		}
		if caminho == "" && !strings.HasSuffix(r.URL.Path, "/") {
			http.Redirect(w, r, r.URL.Path+"/", http.StatusFound)
			return
		}

		switch {
		case caminho == "api/ao-vivo":
			lista, err := transmissoesAoVivo()
			if err != nil {
				http.Error(w, "servidor fora do ar", http.StatusServiceUnavailable)
				return
			}
			w.Header().Set("Content-Type", "application/json")
			w.Header().Set("Cache-Control", "no-store")
			json.NewEncoder(w).Encode(lista)

		case reSinalizacao.MatchString(caminho):
			// WHIP (transmitir) e WHEP (assistir): repassa ao MediaMTX sem o prefixo.
			r.Body = http.MaxBytesReader(w, r.Body, 256<<10)
			prefixo := "/s/" + token
			proxy := &httputil.ReverseProxy{
				Rewrite: func(pr *httputil.ProxyRequest) {
					pr.SetURL(mtx)
					pr.Out.URL.Path = "/" + caminho
					pr.Out.URL.RawPath = ""
					pr.Out.Header.Del("X-Forwarded-For")
					pr.Out.Header.Del("Origin")
				},
				ModifyResponse: func(resp *http.Response) error {
					if loc := resp.Header.Get("Location"); strings.HasPrefix(loc, "/") {
						resp.Header.Set("Location", prefixo+loc)
					}
					resp.Header.Del("Access-Control-Allow-Origin")
					return nil
				},
			}
			proxy.ServeHTTP(w, r)

		case r.Method == http.MethodGet && !strings.Contains(caminho, "/"):
			r2 := r.Clone(r.Context())
			r2.URL.Path = "/" + caminho
			w.Header().Set("Cache-Control", "no-cache")
			estaticos.ServeHTTP(w, r2)

		default:
			http.NotFound(w, r)
		}
	})
}

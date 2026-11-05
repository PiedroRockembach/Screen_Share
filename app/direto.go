package main

// Teste da conexão direta (SRT por UDP, sem navegador e sem Funnel). Roda no
// console quando o executável se chama TesteDireto/teste-direto. Serve para
// validar rede, qualidade e áudio sem o Discord antes de construir o app dedicado.

import (
	"bufio"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

const (
	portaSRT     = 48890 // UDP: vídeo e som
	portaAPISRT  = 48891 // API do MediaMTX, só 127.0.0.1
	nomeHost     = "anfitriao"
	nomeConvid   = "amigo"
	prefixoTeste = "TESTE-"
)

func modoTesteDireto() bool {
	nome := strings.ToLower(filepath.Base(os.Args[0]))
	return strings.Contains(nome, "testedireto") || strings.Contains(nome, "teste-direto")
}

const configSRT = `
logLevel: info
rtsp: false
rtmp: false
hls: false
webrtc: false
moq: false
api: true
apiAddress: 127.0.0.1:%d
# O túnel do Tailscale tem MTU 1280: 1128 bytes (6 pacotes MPEG-TS) sempre cabem.
udpMaxPayloadSize: 1128
srt: true
srtAddress: %s:%d
authInternalUsers:
  - user: any
    pass:
    ips: []
    permissions:
      - action: publish
      - action: read
  - user: any
    pass:
    ips: ["127.0.0.1", "::1"]
    permissions:
      - action: api
paths:
  all_others:
    srtPublishPassphrase: %s
    srtReadPassphrase: %s
`

func uriSRT(ip, senha, streamid string) string {
	return fmt.Sprintf("srt://%s:%d?mode=caller&streamid=%s&passphrase=%s&latency=200", ip, portaSRT, streamid, senha)
}

// finalEnvio é igual nos dois sistemas: H.264 + Opus em MPEG-TS sobre SRT.
// O servidor só reconhece as faixas anunciadas no começo, e o codificador demora
// a iniciar: o latency do misturador espera o vídeo em vez de anunciar só o som.
// Com sync=false essa espera não vira atraso depois que os dois estão fluindo.
func finalEnvio(uri string) string {
	return " ! h264parse config-interval=-1 ! queue ! mux. mpegtsmux name=mux alignment=6 latency=5000000000 ! srtsink sync=false uri=" + uri
}

func argumentosReproducao(uri string) []string {
	return []string{"playbin", "uri=" + uri}
}

type sessaoTeste struct {
	a             *App
	ip, senha     string
	eu, outro     string
	anfitriao     bool
	envio         *Transmissao
	reproducao    *exec.Cmd
	fimReproducao chan struct{}
	entrada       *bufio.Reader
}

func executarTesteDireto() {
	arq, err := os.Create(filepath.Join(pastaDados, "teste-direto.log"))
	if err == nil {
		log.SetOutput(io.MultiWriter(os.Stdout, arq))
	}
	log.SetFlags(log.Ltime)
	entrada := bufio.NewReader(os.Stdin)
	a := novoApp()
	go mostrarAcoes(a)

	fmt.Println("===== Sala de Tela: teste da conexão direta =====")
	log.Printf("sistema: %s; registros em %s", runtime.GOOS, pastaDados)
	fmt.Println("1) Eu vou HOSPEDAR o teste")
	fmt.Println("2) Eu recebi um código TESTE- de quem hospeda")
	var s *sessaoTeste
	if perguntar(entrada, "Digite 1 ou 2: ") == "1" {
		s, err = hospedarTeste(a, entrada)
	} else {
		s, err = entrarTeste(a, entrada)
	}
	if err != nil {
		log.Printf("ERRO: %v", err)
		perguntar(entrada, "Aperte Enter para fechar.")
		return
	}
	s.entrada = entrada
	s.laco()
}

func perguntar(r *bufio.Reader, texto string) string {
	fmt.Print(texto)
	linha, _ := r.ReadString('\n')
	return strings.TrimSpace(linha)
}

// mostrarAcoes imprime no console o que no painel apareceria como aviso.
func mostrarAcoes(a *App) {
	ultimo := ""
	for {
		a.mu.Lock()
		atual := strings.TrimSpace(a.textoAcao + " " + a.urlAcao)
		a.mu.Unlock()
		if atual != ultimo && atual != "" {
			log.Printf(">>> %s", atual)
		}
		ultimo = atual
		time.Sleep(300 * time.Millisecond)
	}
}

func hospedarTeste(a *App, entrada *bufio.Reader) (*sessaoTeste, error) {
	if _, err := a.garantirTailscale(); err != nil {
		return nil, err
	}
	ip, err := ipParaRelay()
	if err != nil {
		return nil, err
	}
	if err := a.garantirMediaMTX(); err != nil {
		return nil, err
	}
	if err := liberarFirewallSRT(a); err != nil {
		log.Printf("AVISO: não liberei o firewall (%v); a conexão pode falhar", err)
	}
	senha := aleatorio(12)
	if err := iniciarServidorSRT(ip, senha); err != nil {
		return nil, err
	}
	log.Printf("servidor pronto em %s:%d (UDP)", ip, portaSRT)

	fmt.Println()
	fmt.Println("Agora crie o convite. Vou abrir a lista de máquinas do Tailscale:")
	fmt.Println("  na linha deste PC: menu (...) > Share > Copy invite link > ligue \"Reusable link\".")
	abrirNavegador("https://login.tailscale.com/admin/machines")
	convite := perguntar(entrada, "Cole aqui o link de convite (Enter se o amigo já tem acesso): ")

	codigo := prefixoTeste + base64.StdEncoding.EncodeToString([]byte(ip.String()+"|"+senha+"|"+convite))
	fmt.Printf("\nMande este código para o seu amigo em mensagem privada:\n\n%s\n\n", codigo)
	return &sessaoTeste{a: a, ip: ip.String(), senha: senha, eu: nomeHost, outro: nomeConvid, anfitriao: true}, nil
}

func iniciarServidorSRT(ip net.IP, senha string) error {
	conf := filepath.Join(pastaDados, "teste-direto.yml")
	texto := fmt.Sprintf(configSRT, portaAPISRT, ip, portaSRT, senha, senha)
	if err := os.WriteFile(conf, []byte(texto), 0o600); err != nil {
		return err
	}
	cmd := exec.Command(binarioMediaMTX(), conf)
	cmd.Dir = pastaDados
	if f, err := os.Create(filepath.Join(pastaDados, "teste-direto-mediamtx.log")); err == nil {
		cmd.Stdout, cmd.Stderr = f, f
	}
	prepararFilho(cmd)
	if err := cmd.Start(); err != nil {
		return fmt.Errorf("iniciar o MediaMTX: %w", err)
	}
	vincularFilho(cmd)
	go func() {
		err := cmd.Wait()
		log.Printf("ERRO: o servidor MediaMTX fechou (%v); veja teste-direto-mediamtx.log", err)
	}()
	for i := 0; i < 20; i++ {
		if _, err := estatisticasSRT(); err == nil {
			go registrarEstatisticas()
			return nil
		}
		time.Sleep(500 * time.Millisecond)
	}
	return errors.New("o MediaMTX não respondeu (veja teste-direto-mediamtx.log)")
}

func entrarTeste(a *App, entrada *bufio.Reader) (*sessaoTeste, error) {
	codigo := strings.TrimPrefix(strings.ReplaceAll(perguntar(entrada, "Cole o código: "), " ", ""), prefixoTeste)
	bruto, err := base64.StdEncoding.DecodeString(codigo)
	if err != nil {
		return nil, errors.New("esse código não é válido: copie a mensagem inteira")
	}
	partes := strings.SplitN(string(bruto), "|", 3)
	if len(partes) != 3 || net.ParseIP(partes[0]) == nil {
		return nil, errors.New("esse código não é válido: copie a mensagem inteira")
	}
	if _, err := a.garantirTailscale(); err != nil {
		return nil, err
	}
	ip, senha, convite := partes[0], partes[1], partes[2]
	log.Print("verificando se este PC já alcança quem hospeda…")
	if !alcanca(ip) {
		if convite != "" {
			log.Print(">>> Aceite o convite na página que abriu; depois é só esperar aqui.")
			abrirNavegador(convite)
		}
		limite := time.Now().Add(10 * time.Minute)
		for !alcanca(ip) {
			if time.Now().After(limite) {
				return nil, errors.New("não alcancei quem hospeda em 10 minutos: o convite foi aceito? O PC dele está ligado?")
			}
		}
	}
	log.Print("conectado a quem hospeda!")
	return &sessaoTeste{a: a, ip: ip, senha: senha, eu: nomeConvid, outro: nomeHost}, nil
}

func alcanca(ip string) bool {
	return comandoTS("ping", "--c=1", "--timeout=5s", ip).Run() == nil
}

func (s *sessaoTeste) laco() {
	for {
		fmt.Println()
		fmt.Println("[t] transmitir/parar   [a] assistir/parar   [c] testar a conexão")
		if s.anfitriao {
			fmt.Println("[e] estatísticas do servidor")
		}
		fmt.Println("[d] diagnóstico        [q] sair")
		if s.envio != nil {
			if ativa, status := s.envio.estado(); ativa {
				log.Printf("transmissão: %s", status)
			}
		}
		switch strings.ToLower(perguntar(s.entrada, "> ")) {
		case "t":
			s.alternarEnvio()
		case "a":
			s.alternarReproducao()
		case "c":
			s.testarConexao()
		case "e":
			if s.anfitriao {
				s.mostrarEstatisticas()
			}
		case "d":
			diagnosticoDireto(s.a)
			log.Printf("arquivos para enviar se algo falhar: %s", pastaDados)
		case "q":
			s.encerrar()
			return
		}
	}
}

func (s *sessaoTeste) alternarEnvio() {
	if s.envio != nil {
		if ativa, _ := s.envio.estado(); ativa {
			s.envio.parar()
			s.envio = nil
			log.Print("transmissão parada")
			return
		}
	}
	comSom := !strings.HasPrefix(strings.ToLower(perguntar(s.entrada, "Com som? (Enter = sim, n = não): ")), "n")
	t, err := s.a.enviarDireto(uriSRT(s.ip, s.senha, "publish:"+s.eu), comSom)
	if err != nil {
		log.Printf("ERRO ao transmitir: %v", err)
		return
	}
	s.envio = t
	time.Sleep(2 * time.Second)
	_, status := t.estado()
	log.Printf("transmissão: %s", status)
}

func (s *sessaoTeste) alternarReproducao() {
	if s.reproducao != nil {
		select {
		case <-s.fimReproducao:
		default:
			s.reproducao.Process.Kill()
			<-s.fimReproducao
			s.reproducao = nil
			log.Print("reprodução parada")
			return
		}
	}
	cmd, err := s.a.comandoGst(argumentosReproducao(uriSRT(s.ip, s.senha, "read:"+s.outro))...)
	if err != nil {
		log.Printf("ERRO ao assistir: %v", err)
		return
	}
	if f, err := os.Create(filepath.Join(pastaDados, "teste-direto-reproducao.log")); err == nil {
		cmd.Stdout, cmd.Stderr = f, f
	}
	if err := cmd.Start(); err != nil {
		log.Printf("ERRO ao assistir: %v", err)
		return
	}
	vincularFilho(cmd)
	fim := make(chan struct{})
	go func() {
		err := cmd.Wait()
		close(fim)
		log.Printf("a reprodução terminou (%v). Se %s não estava transmitindo, é normal.", err, s.outro)
	}()
	s.reproducao, s.fimReproducao = cmd, fim
	log.Printf("abrindo o vídeo de %s…", s.outro)
}

func (s *sessaoTeste) testarConexao() {
	log.Print("testando… (\"via DERP\" = retransmissor do Tailscale; com IP e porta = conexão direta)")
	saida, _ := comandoTS("ping", "--c=6", "--timeout=5s", s.ip).CombinedOutput()
	log.Print(strings.TrimSpace(string(saida)))
}

func estatisticasSRT() (string, error) {
	cli := http.Client{Timeout: 3 * time.Second}
	resp, err := cli.Get(fmt.Sprintf("http://127.0.0.1:%d/v3/srtconns/list", portaAPISRT))
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	var dados struct {
		Items []map[string]any `json:"items"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&dados); err != nil {
		return "", err
	}
	var b strings.Builder
	for _, c := range dados.Items {
		fmt.Fprintf(&b, "%v %v: RTT %.0f ms, recebendo %.1f Mbps, enviando %.1f Mbps, perdidos %v, reenviados %v\n",
			c["state"], c["path"], numero(c["msRTT"]), numero(c["mbpsReceiveRate"]), numero(c["mbpsSendRate"]),
			c["packetsReceivedLoss"], c["packetsSendRetrans"])
	}
	return b.String(), nil
}

func numero(v any) float64 {
	f, _ := v.(float64)
	return f
}

func registrarEstatisticas() {
	for range time.Tick(10 * time.Second) {
		if e, err := estatisticasSRT(); err == nil && e != "" {
			registrarSoNoArquivo(e)
		}
	}
}

func registrarSoNoArquivo(texto string) {
	if f, err := os.OpenFile(filepath.Join(pastaDados, "teste-direto-estatisticas.log"), os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644); err == nil {
		fmt.Fprintf(f, "%s\n%s", time.Now().Format("15:04:05"), texto)
		f.Close()
	}
}

func (s *sessaoTeste) mostrarEstatisticas() {
	e, err := estatisticasSRT()
	if err != nil {
		log.Printf("sem estatísticas: %v", err)
		return
	}
	if e == "" {
		e = "ninguém conectado"
	}
	log.Print(strings.TrimSpace(e))
}

func (s *sessaoTeste) encerrar() {
	if s.envio != nil {
		s.envio.parar()
	}
	if s.reproducao != nil && s.reproducao.Process != nil {
		s.reproducao.Process.Kill()
	}
}

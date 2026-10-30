package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os/exec"
	"regexp"
	"runtime"
	"strings"
	"time"
)

type estadoTS struct {
	BackendState string
	AuthURL      string
	Self         struct {
		DNSName string
		Online  bool
	}
	CurrentTailnet *struct {
		MagicDNSEnabled bool
	}
}

var errDaemonParado = errors.New("o serviço do Tailscale não está rodando")

func comandoTS(args ...string) *exec.Cmd {
	cmd := exec.Command(binarioTailscale(), args...)
	prepararFilho(cmd)
	return cmd
}

func statusTS() (*estadoTS, error) {
	if binarioTailscale() == "" {
		return nil, errors.New("Tailscale não instalado")
	}
	saida, err := comandoTS("status", "--json").Output()
	if len(saida) == 0 || saida[0] != '{' {
		if err != nil {
			return nil, errDaemonParado
		}
	}
	var st estadoTS
	if errJSON := json.Unmarshal(saida, &st); errJSON != nil {
		return nil, fmt.Errorf("status do Tailscale: %w", errJSON)
	}
	return &st, nil
}

var reURL = regexp.MustCompile(`https://\S+`)

// garantirTailscale deixa o Tailscale instalado, rodando e logado.
// Devolve o endereço público do PC (ex.: piedro-desktop.tail1234.ts.net).
func (a *App) garantirTailscale() (string, error) {
	if err := a.prepararTailscale(); err != nil {
		return "", err
	}
	st, err := statusTS()
	if err != nil {
		return "", err
	}
	if st.BackendState != "Running" {
		if err := a.loginTailscale(st); err != nil {
			return "", err
		}
		if st, err = statusTS(); err != nil {
			return "", err
		}
	}
	if st.CurrentTailnet != nil && !st.CurrentTailnet.MagicDNSEnabled {
		a.definirAcao("Ative o MagicDNS na sua conta do Tailscale e crie a sala de novo", "https://login.tailscale.com/admin/dns")
		return "", errors.New("o MagicDNS está desligado na sua conta do Tailscale (é um clique: botão \"Enable MagicDNS\")")
	}
	dns := strings.TrimSuffix(st.Self.DNSName, ".")
	if dns == "" {
		return "", errors.New("o Tailscale não informou o nome deste PC")
	}
	return dns, nil
}

func (a *App) loginTailscale(st *estadoTS) error {
	a.registrar("entrando no Tailscale…")
	var cmd *exec.Cmd
	if st.BackendState == "NeedsLogin" || st.BackendState == "NoState" {
		cmd = comandoTS("login")
	} else {
		args := []string{"up"}
		if runtime.GOOS == "linux" {
			args = append(args, "--operator="+usuarioDoSistema())
		}
		cmd = comandoTS(args...)
	}
	saida, _ := cmd.StdoutPipe()
	cmd.Stderr = cmd.Stdout
	if err := cmd.Start(); err != nil {
		return err
	}
	vincularFilho(cmd)
	var textoSaida strings.Builder
	fim := make(chan error, 1)
	go func() {
		io.Copy(&textoSaida, saida) // ler tudo antes do Wait (ele fecha o pipe)
		fim <- cmd.Wait()
	}()

	avisado := ""
	limite := time.After(15 * time.Minute)
	for {
		select {
		case err := <-fim:
			if st, e := statusTS(); e == nil && st.BackendState == "Running" {
				a.registrar("Tailscale conectado")
				return nil
			}
			// "tailscale up" pede para repetir as opções já usadas: roda a sugestão dele.
			if sugestao := regexp.MustCompile(`(?m)^\s*tailscale up (.*)$`).FindStringSubmatch(textoSaida.String()); sugestao != nil {
				if err := comandoTS(append([]string{"up"}, strings.Fields(sugestao[1])...)...).Run(); err == nil {
					return nil
				}
			}
			return fmt.Errorf("login no Tailscale falhou: %v %s", err, strings.TrimSpace(textoSaida.String()))
		case <-limite:
			cmd.Process.Kill()
			return errors.New("o login no Tailscale não foi concluído a tempo")
		case <-time.After(time.Second):
			if st, err := statusTS(); err == nil {
				if st.BackendState == "Running" {
					cmd.Process.Kill()
					a.registrar("Tailscale conectado")
					return nil
				}
				if st.AuthURL != "" && st.AuthURL != avisado {
					avisado = st.AuthURL
					a.definirAcao("Entre na sua conta do Tailscale (pode ser com o Google) na página que abriu", st.AuthURL)
				}
			}
		}
	}
}

type estadoFunnel struct {
	AllowFunnel map[string]bool
	Foreground  map[string]struct {
		AllowFunnel map[string]bool
	}
}

func funnelPermitido(b []byte, dns string) bool {
	var estado estadoFunnel
	if json.Unmarshal(b, &estado) != nil {
		return false
	}
	if estado.AllowFunnel[dns+":443"] {
		return true
	}
	for _, foreground := range estado.Foreground {
		if foreground.AllowFunnel[dns+":443"] {
			return true
		}
	}
	return false
}

func dnsPublicoResolve(dns string) bool {
	resolver := &net.Resolver{
		PreferGo: true,
		Dial: func(ctx context.Context, network, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "udp", "1.1.1.1:53")
		},
	}
	ctx, cancelar := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancelar()
	enderecos, err := resolver.LookupHost(ctx, dns)
	return err == nil && len(enderecos) > 0
}

// iniciarFunnel publica a porta local na internet em https://<dns>/.
// O processo fica em primeiro plano: quando ele morre, o Funnel some sozinho.
func (a *App) iniciarFunnel(dns string, porta int) (*exec.Cmd, error) {
	cmd := comandoTS("funnel", fmt.Sprint(porta))
	saida, _ := cmd.StdoutPipe()
	cmd.Stderr = cmd.Stdout
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	vincularFilho(cmd)
	morreu := make(chan struct{})
	var ultimas []string
	go func() {
		sc := bufio.NewScanner(saida)
		for sc.Scan() {
			linha := sc.Text()
			ultimas = append(ultimas, linha)
			// Primeira vez: o Tailscale pede para autorizar o Funnel/HTTPS na conta.
			if u := reURL.FindString(linha); u != "" && !strings.Contains(u, dns) {
				a.definirAcao("Autorize o Funnel na página do Tailscale que abriu (só na primeira vez)", u)
			}
		}
		cmd.Wait()
		close(morreu)
	}()

	limite := time.After(10 * time.Minute)
	for {
		select {
		case <-morreu:
			return nil, fmt.Errorf("o Funnel do Tailscale fechou: %s", strings.Join(ultimas, " | "))
		case <-limite:
			cmd.Process.Kill()
			return nil, errors.New("o Funnel não foi autorizado a tempo")
		case <-time.After(time.Second):
			b, err := comandoTS("funnel", "status", "--json").Output()
			if err != nil {
				continue
			}
			if funnelPermitido(b, dns) {
				a.definirAcao("aguardando a publicação do endereço no DNS público (pode levar até 10 minutos)", "")
				if dnsPublicoResolve(dns) {
					a.definirAcao("", "")
					return cmd, nil
				}
			}
		}
	}
}

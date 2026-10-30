package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"syscall"
)

func binarioTailscale() string {
	if p, err := exec.LookPath("tailscale"); err == nil {
		return p
	}
	return ""
}

func kernelAtual() string {
	var u syscall.Utsname
	if syscall.Uname(&u) != nil {
		return ""
	}
	var b strings.Builder
	for _, c := range u.Release {
		if c == 0 {
			break
		}
		b.WriteByte(byte(c))
	}
	return b.String()
}

// prepararTailscale instala/liga o Tailscale e libera o usuário para usá-lo.
// Tudo que precisa de root vai num único pedido de senha (pkexec).
func (a *App) prepararTailscale() error {
	if k := kernelAtual(); k != "" {
		if _, err := os.Stat("/usr/lib/modules/" + k); err != nil {
			return errors.New("o kernel foi atualizado e o PC ainda não foi reiniciado: reinicie o PC e tente de novo")
		}
	}
	usuario := usuarioDoSistema()
	var root []string

	if binarioTailscale() == "" {
		if _, err := exec.LookPath("pacman"); err == nil {
			root = append(root, "pacman -S --needed --noconfirm tailscale")
		} else {
			root = append(root, "curl -fsSL https://tailscale.com/install.sh | sh")
		}
	}
	if _, err := statusTS(); err != nil {
		root = append(root,
			"systemctl enable --now tailscaled",
			"for i in $(seq 20); do [ -S /run/tailscale/tailscaled.sock ] && break; sleep 1; done")
	}
	if operadorTS() != usuario {
		root = append(root, "tailscale set --operator="+usuario)
	}
	cfg := a.config()
	if !cfg.FirewallLiberado {
		// O vídeo chega por UDP direto neste PC; o ufw bloquearia.
		root = append(root, fmt.Sprintf("if command -v ufw >/dev/null && ufw status | grep -q 'Status: active'; then ufw allow %d/udp comment 'Sala de Tela'; fi", portaMidia))
	}
	if len(root) == 0 {
		return nil
	}

	a.definirAcao("Digite a sua senha na janela que abriu (só na primeira vez)", "")
	a.registrar("configurando o Tailscale (pede a senha do sistema)…")
	script := "set -e\n" + strings.Join(root, "\n")
	saida, err := exec.Command("pkexec", "sh", "-c", script).CombinedOutput()
	a.definirAcao("", "")
	if err != nil {
		return fmt.Errorf("configuração do Tailscale falhou (senha cancelada?): %v %s", err, strings.TrimSpace(string(saida)))
	}
	a.alterarConfig(func(c *Config) { c.FirewallLiberado = true })
	if _, err := statusTS(); err != nil {
		return err
	}
	return nil
}

func operadorTS() string {
	saida, err := comandoTS("debug", "prefs").Output()
	if err != nil {
		return ""
	}
	var p struct{ OperatorUser string }
	_ = json.Unmarshal(saida, &p)
	return p.OperatorUser
}

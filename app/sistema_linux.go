package main

import (
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
)

func diretorioDados() string {
	if d := os.Getenv("XDG_DATA_HOME"); d != "" {
		return filepath.Join(d, "sala-de-tela")
	}
	casa, _ := os.UserHomeDir()
	return filepath.Join(casa, ".local", "share", "sala-de-tela")
}

func abrirNavegador(url string) {
	if os.Getenv("SALA_DEV") != "" {
		return
	}
	cmd := exec.Command("xdg-open", url)
	cmd.Stdout, cmd.Stderr = nil, nil
	_ = cmd.Start()
	go cmd.Wait()
}

// prepararFilho faz o processo filho morrer junto se o Sala de Tela fechar.
func prepararFilho(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Pdeathsig: syscall.SIGTERM, Setpgid: true}
}

func vincularFilho(*exec.Cmd) {}

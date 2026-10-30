package main

import (
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func binarioTailscale() string {
	p := filepath.Join(os.Getenv("ProgramFiles"), "Tailscale", "tailscale.exe")
	if _, err := os.Stat(p); err == nil {
		return p
	}
	if p, err := exec.LookPath("tailscale"); err == nil {
		return p
	}
	return ""
}

// prepararTailscale instala o Tailscale oficial (MSI, com checksum) se precisar.
func (a *App) prepararTailscale() error {
	if binarioTailscale() == "" {
		info, err := lerURL("https://pkgs.tailscale.com/stable/?mode=json")
		if err != nil {
			return err
		}
		var pkgs struct{ MSIs map[string]string }
		if err := json.Unmarshal([]byte(info), &pkgs); err != nil || pkgs.MSIs["amd64"] == "" {
			return errors.New("não consegui descobrir a versão atual do Tailscale")
		}
		url := "https://pkgs.tailscale.com/stable/" + pkgs.MSIs["amd64"]
		msi := filepath.Join(pastaDados, "downloads", pkgs.MSIs["amd64"])
		a.registrar("baixando o Tailscale…")
		if err := a.baixarVerificado(url, url+".sha256", msi); err != nil {
			return err
		}
		a.definirAcao("Clique em \"Sim\" no aviso do Windows para instalar o Tailscale", "")
		err = comoAdministrador("msiexec.exe", `/i "`+msi+`" /quiet /norestart TS_NOLAUNCH=1`)
		a.definirAcao("", "")
		if err != nil {
			return err
		}
		os.Remove(msi)
	}
	for i := 0; i < 30; i++ {
		if binarioTailscale() != "" {
			if _, err := statusTS(); err == nil {
				return nil
			}
		}
		time.Sleep(time.Second)
	}
	return errDaemonParado
}

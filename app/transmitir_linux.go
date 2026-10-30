package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
)

// Transmissão no Linux: o OBS (captura PipeWire do Wayland/X11 e encoder de
// hardware) envia via WHIP. O app gera um perfil "Sala de Tela" no OBS a cada
// transmissão e captura só o áudio da "sala_de_tela" (sem o Discord).

const nomeOBS = "Sala de Tela"

type instalacaoOBS struct {
	cmd    []string
	config string
}

func acharOBS() (*instalacaoOBS, error) {
	casa, _ := os.UserHomeDir()
	if p, err := exec.LookPath("obs"); err == nil {
		return &instalacaoOBS{[]string{p}, filepath.Join(casa, ".config", "obs-studio")}, nil
	}
	if exec.Command("flatpak", "info", "com.obsproject.Studio").Run() == nil {
		return &instalacaoOBS{[]string{"flatpak", "run", "com.obsproject.Studio"},
			filepath.Join(casa, ".var", "app", "com.obsproject.Studio", "config", "obs-studio")}, nil
	}
	return nil, errors.New("instale o OBS Studio para transmitir (no CachyOS/Arch: sudo pacman -S obs-studio)")
}

func encoderOBS() (id string, ajustes map[string]any, nome string) {
	if _, err := os.Stat("/proc/driver/nvidia/version"); err == nil {
		return "obs_nvenc_h264_tex", map[string]any{
			"rate_control": "CBR", "bitrate": 9000, "keyint_sec": 1, "preset2": "p5", "tune": "ll",
			"multipass": "disabled", "profile": "high", "bf": 0, "lookahead": false,
		}, "NVIDIA NVENC"
	}
	if _, err := os.Stat("/dev/dri/renderD128"); err == nil {
		return "ffmpeg_vaapi_tex", map[string]any{
			"rate_control": "CBR", "bitrate": 9000, "keyint_sec": 1, "bf": 0, "profile": 100,
			"vaapi_device": "/dev/dri/renderD128",
		}, "VAAPI (AMD/Intel)"
	}
	return "obs_x264", map[string]any{
		"rate_control": "CBR", "bitrate": 6000, "keyint_sec": 1, "preset": "veryfast",
		"tune": "zerolatency", "x264opts": "bframes=0",
	}, "x264 (processador)"
}

func escreverJSON(caminho string, v any) error {
	if err := os.MkdirAll(filepath.Dir(caminho), 0o755); err != nil {
		return err
	}
	b, _ := json.MarshalIndent(v, "", "  ")
	return os.WriteFile(caminho, b, 0o644)
}

// gerarPerfilOBS cria/atualiza o perfil e as cenas "Sala de Tela" sem mexer nos outros.
func gerarPerfilOBS(base, whip string, comSom bool) (string, error) {
	id, ajustes, nomeEnc := encoderOBS()
	perfil := filepath.Join(base, "basic", "profiles", "SalaDeTela")
	if err := os.MkdirAll(perfil, 0o755); err != nil {
		return "", err
	}
	ini := fmt.Sprintf(`[General]
Name=%s

[Output]
Mode=Advanced

[AdvOut]
Encoder=%s
AudioEncoder=ffmpeg_opus
TrackIndex=1
RecEncoder=none

[Video]
BaseCX=1920
BaseCY=1080
OutputCX=1920
OutputCY=1080
FPSType=0
FPSCommon=60
ScaleType=bicubic
ColorFormat=NV12
ColorSpace=709
ColorRange=Partial

[Audio]
SampleRate=48000
ChannelSetup=Stereo
`, nomeOBS, id)
	if err := os.WriteFile(filepath.Join(perfil, "basic.ini"), []byte(ini), 0o644); err != nil {
		return "", err
	}
	if err := escreverJSON(filepath.Join(perfil, "streamEncoder.json"), ajustes); err != nil {
		return "", err
	}
	if err := escreverJSON(filepath.Join(perfil, "service.json"), map[string]any{
		"type":     "whip_custom",
		"settings": map[string]any{"service": "WHIP", "server": whip, "bearer_token": ""},
	}); err != nil {
		return "", err
	}

	// Mantém a permissão de captura de tela já dada (senão o KDE pergunta sempre).
	cenas := filepath.Join(base, "basic", "scenes", "SalaDeTela.json")
	token := ""
	if b, err := os.ReadFile(cenas); err == nil {
		var antigo struct {
			Sources []struct {
				ID       string         `json:"id"`
				Settings map[string]any `json:"settings"`
			} `json:"sources"`
		}
		if json.Unmarshal(b, &antigo) == nil {
			for _, s := range antigo.Sources {
				if t, ok := s.Settings["RestoreToken"].(string); ok && s.ID == "pipewire-screen-capture-source" {
					token = t
				}
			}
		}
	}
	idTela, idCena := aleatorioUUID(), aleatorioUUID()
	ajustesTela := map[string]any{"ShowCursor": true}
	if token != "" {
		ajustesTela["RestoreToken"] = token
	}
	colecao := map[string]any{
		"name": nomeOBS,
		"sources": []any{
			map[string]any{"id": "pipewire-screen-capture-source", "versioned_id": "pipewire-screen-capture-source",
				"name": "Minha tela", "uuid": idTela, "settings": ajustesTela, "enabled": true},
			map[string]any{"id": "scene", "versioned_id": "scene", "name": "Sala", "uuid": idCena, "enabled": true,
				"settings": map[string]any{"id_counter": 1, "custom_size": false, "items": []any{map[string]any{
					"name": "Minha tela", "source_uuid": idTela, "visible": true, "locked": true, "rot": 0.0, "align": 5,
					// Escala qualquer tela (4K, ultrawide...) para caber em 1920x1080.
					"bounds_type": 2, "bounds_align": 0, "bounds": map[string]any{"x": 1920.0, "y": 1080.0},
					"pos": map[string]any{"x": 0.0, "y": 0.0}, "scale": map[string]any{"x": 1.0, "y": 1.0}, "id": 1,
				}}}},
		},
		"groups":                []any{},
		"scene_order":           []any{map[string]any{"name": "Sala"}},
		"current_scene":         "Sala",
		"current_program_scene": "Sala",
		"transitions":           []any{},
		"current_transition":    "Fade",
		"transition_duration":   300,
	}
	if comSom {
		// Só a "sala_de_tela" como áudio; sem microfone (AuxAudioDevice).
		colecao["DesktopAudioDevice1"] = map[string]any{
			"id": "pulse_output_capture", "versioned_id": "pulse_output_capture",
			"name": "Áudio da Sala (sem Discord)", "uuid": aleatorioUUID(),
			"settings": map[string]any{"device_id": sinkSala + ".monitor"},
			"mixers":   255, "volume": 1.0, "enabled": true, "muted": false, "monitoring_type": 0,
		}
	}
	return nomeEnc, escreverJSON(cenas, colecao)
}

func aleatorioUUID() string {
	h := aleatorio(16)
	return h[0:8] + "-" + h[8:12] + "-4" + h[13:16] + "-a" + h[17:20] + "-" + h[20:32]
}

func (a *App) transmitirSO(whip, nome string, comSom bool) (*Transmissao, error) {
	obs, err := acharOBS()
	if err != nil {
		return nil, err
	}
	if exec.Command("pgrep", "-x", "obs").Run() == nil {
		return nil, errors.New("o OBS já está aberto: feche ele e clique em Transmitir de novo")
	}
	t := novaTransmissao()

	aviso := "sem som (desmarcado)"
	if comSom {
		if iniciarAudioSala(t.fim) {
			aviso = "com o som do PC, sem o Discord"
		} else {
			comSom = false
			aviso = "SEM som (não consegui criar a saída de áudio da Sala)"
		}
	}
	nomeEnc, err := gerarPerfilOBS(obs.config, whip, comSom)
	if err != nil {
		t.cancelar()
		return nil, err
	}
	args := append(obs.cmd[1:], "--profile", nomeOBS, "--collection", nomeOBS, "--startstreaming")
	cmd := exec.Command(obs.cmd[0], args...)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if f, err := os.Create(filepath.Join(pastaDados, "obs.log")); err == nil {
		cmd.Stdout, cmd.Stderr = f, f
	}
	if err := cmd.Start(); err != nil {
		t.cancelar()
		return nil, fmt.Errorf("abrir o OBS: %w", err)
	}
	t.definir(fmt.Sprintf("AO VIVO pelo OBS como \"%s\" (1920x1080 60fps, %s), %s. Na primeira vez, escolha a tela no aviso do sistema.", nome, nomeEnc, aviso))

	go func() {
		defer close(t.parada)
		saiu := make(chan struct{})
		go func() { cmd.Wait(); close(saiu) }()
		select {
		case <-t.fim:
			cmd.Process.Signal(syscall.SIGTERM)
			<-saiu
		case <-saiu:
			t.cancelar() // fecharam o OBS: desmonta o áudio da Sala
		}
	}()
	return t, nil
}

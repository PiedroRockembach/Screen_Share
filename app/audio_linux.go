package main

import (
	"bufio"
	"encoding/json"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// Áudio no Linux sem o Discord:
//   - cria a saída virtual "sala_de_tela", que repassa o som para a saída padrão
//     (você continua ouvindo tudo);
//   - move todo app que toca na saída padrão para a "sala_de_tela", MENOS o Discord;
//   - o OBS captura só o monitor da "sala_de_tela".
// Se algo falhar, os apps ficam na saída padrão e só não vão para a
// transmissão: a voz do Discord nunca entra. Tudo some quando a transmissão acaba.

const sinkSala = "sala_de_tela"

type entradaPA struct {
	Index      int
	Sink       int
	Properties map[string]string
}

func pactlJSON(v any, args ...string) error {
	saida, err := exec.Command("pactl", append([]string{"-f", "json"}, args...)...).Output()
	if err != nil {
		return err
	}
	return json.Unmarshal(saida, v)
}

func executaveisAncestrais(pid int) []string {
	var nomes []string
	for i := 0; pid > 1 && i < 32; i++ {
		cmdline, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/cmdline")
		if err != nil {
			break
		}
		argv0, _, _ := strings.Cut(string(cmdline), "\x00")
		nomes = append(nomes, argv0[strings.LastIndex(argv0, "/")+1:])
		stat, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/stat")
		if err != nil {
			break
		}
		// o nome pode ter espaços e parênteses: o ppid vem depois do último ')'
		depois := string(stat[strings.LastIndex(string(stat), ")")+1:])
		campos := strings.Fields(depois)
		if len(campos) < 2 {
			break
		}
		pid, _ = strconv.Atoi(campos[1])
	}
	return nomes
}

func ehDiscordPA(props map[string]string) bool {
	for k, v := range props {
		if (strings.HasPrefix(k, "application.") || k == "pipewire.access.portal.app_id") && reDiscord.MatchString(v) {
			return true
		}
	}
	pid, _ := strconv.Atoi(props["application.process.id"])
	for _, nome := range executaveisAncestrais(pid) {
		if reDiscord.MatchString(nome) {
			return true
		}
	}
	return false
}

func ajustarAudio() {
	var sinks []struct {
		Index int
		Name  string
	}
	if pactlJSON(&sinks, "list", "sinks") != nil {
		return
	}
	nomes := map[int]string{}
	for _, s := range sinks {
		nomes[s.Index] = s.Name
	}
	saida, err := exec.Command("pactl", "get-default-sink").Output()
	padrao := strings.TrimSpace(string(saida))
	if err != nil || padrao == "" || padrao == sinkSala {
		return
	}
	var entradas []entradaPA
	if pactlJSON(&entradas, "list", "sink-inputs") != nil {
		return
	}
	for _, e := range entradas {
		p := e.Properties
		if strings.HasPrefix(p["node.name"], "sala_") || p["node.link-group"] != "" {
			continue // loopbacks e filtros não são apps
		}
		onde := nomes[e.Sink]
		discord := ehDiscordPA(p)
		switch {
		case discord && onde == sinkSala:
			exec.Command("pactl", "move-sink-input", strconv.Itoa(e.Index), padrao).Run()
		case !discord && onde == padrao:
			exec.Command("pactl", "move-sink-input", strconv.Itoa(e.Index), sinkSala).Run()
		}
	}
}

// iniciarAudioSala cria a "sala_de_tela" e mantém o roteamento até fim fechar.
// Devolve false se não deu para montar (então transmitimos sem som).
func iniciarAudioSala(fim <-chan struct{}) bool {
	for _, p := range []string{"pactl", "pw-loopback"} {
		if _, err := exec.LookPath(p); err != nil {
			return false
		}
	}
	loop := exec.Command("pw-loopback",
		"--capture-props=node.name="+sinkSala+" node.description=Sala-de-Tela media.class=Audio/Sink audio.position=[FL,FR] priority.session=1 priority.driver=1",
		"--playback-props=node.name=sala_de_tela_saida audio.position=[FL,FR] node.passive=true")
	prepararFilho(loop)
	if loop.Start() != nil {
		return false
	}
	pronto := false
	for i := 0; i < 20 && !pronto; i++ {
		time.Sleep(100 * time.Millisecond)
		var sinks []struct{ Name string }
		pactlJSON(&sinks, "list", "sinks")
		for _, s := range sinks {
			pronto = pronto || s.Name == sinkSala
		}
	}
	if !pronto {
		loop.Process.Kill()
		loop.Wait()
		return false
	}

	go func() {
		ajustarAudio()
		eventos := exec.Command("pactl", "subscribe")
		prepararFilho(eventos)
		saida, _ := eventos.StdoutPipe()
		if eventos.Start() == nil {
			go func() {
				sc := bufio.NewScanner(saida)
				for sc.Scan() {
					l := sc.Text()
					if strings.Contains(l, "sink-input") || strings.Contains(l, "'server'") || strings.Contains(l, "on sink #") {
						ajustarAudio()
					}
				}
			}()
		}
		<-fim
		if eventos.Process != nil {
			eventos.Process.Kill()
			eventos.Wait()
		}
		loop.Process.Kill() // a "sala_de_tela" some e os apps voltam para a saída padrão
		loop.Wait()
	}()
	return true
}

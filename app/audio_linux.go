package main

import (
	"bufio"
	"encoding/json"
	"log"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

// Áudio no Linux sem o Discord:
//   - cria a saída virtual "sala_de_tela", que é muda (não toca em lugar nenhum);
//   - para cada app que toca som, MENOS o Discord, liga uma CÓPIA da saída dele
//     na "sala_de_tela". O app continua tocando onde já tocava (fone, caixa,
//     EasyEffects...), então nada muda no que você ouve;
//   - o OBS captura só o monitor da "sala_de_tela".
// Mover os apps de saída não funciona com o EasyEffects (ele puxa os apps de
// volta na hora); a cópia não briga com ninguém. Se algo falhar, o pior caso é
// um app não ir para a transmissão: a voz do Discord nunca entra.

var sinkSala = "sala_de_tela" // var só para os testes usarem outro nome

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

// Programas de áudio que não são "apps": copiar a saída deles duplicaria o som.
func naoEhApp(props map[string]string) bool {
	if strings.HasPrefix(props["node.name"], "sala_") || props["node.link-group"] != "" {
		return true // loopbacks e filtros
	}
	bin := strings.ToLower(props["application.process.binary"] + " " + props["application.name"] + " " + props["application.id"])
	for _, nome := range []string{"easyeffects", "pulseeffects", "obs"} {
		if strings.Contains(bin, nome) {
			return true
		}
	}
	return false
}

type objetoPW struct {
	ID   int    `json:"id"`
	Type string `json:"type"`
	Info struct {
		Props        map[string]any `json:"props"`
		OutputPortID int            `json:"output-port-id"`
		InputPortID  int            `json:"input-port-id"`
	} `json:"info"`
}

func texto(props map[string]any) map[string]string {
	m := make(map[string]string, len(props))
	for k, v := range props {
		switch x := v.(type) {
		case string:
			m[k] = x
		case float64:
			m[k] = strconv.FormatFloat(x, 'f', -1, 64)
		case bool:
			m[k] = strconv.FormatBool(x)
		}
	}
	return m
}

// lado diz para qual canal da Sala (FL/FR) vai cada canal do app.
func lado(canal string) []string {
	switch canal {
	case "FL", "RL", "SL", "FLC", "RLC":
		return []string{"FL"}
	case "FR", "RR", "SR", "FRC", "RRC":
		return []string{"FR"}
	default: // MONO, FC, LFE...
		return []string{"FL", "FR"}
	}
}

// ajustarAudio liga a cópia de cada app na Sala e garante que o Discord fica fora.
func ajustarAudio() {
	saida, err := exec.Command("pw-dump").Output()
	if err != nil {
		return
	}
	var objetos []objetoPW
	if json.Unmarshal(saida, &objetos) != nil {
		return
	}
	nos := map[int]map[string]string{}
	for _, o := range objetos {
		if o.Type == "PipeWire:Interface:Node" {
			nos[o.ID] = texto(o.Info.Props)
		}
	}
	type porta struct {
		id    int
		no    int
		canal string
	}
	entradaSala := map[string]int{} // canal -> porta de entrada da Sala
	var saidasApps []porta
	for _, o := range objetos {
		if o.Type != "PipeWire:Interface:Port" {
			continue
		}
		p := texto(o.Info.Props)
		no, _ := strconv.Atoi(p["node.id"])
		props := nos[no]
		switch {
		case props["node.name"] == sinkSala && p["port.direction"] == "in":
			entradaSala[p["audio.channel"]] = o.ID
		case props["media.class"] == "Stream/Output/Audio" && p["port.direction"] == "out" && p["port.monitor"] != "true":
			saidasApps = append(saidasApps, porta{o.ID, no, p["audio.channel"]})
		}
	}
	if len(entradaSala) == 0 {
		return
	}
	existe := map[[2]int]bool{}
	for _, o := range objetos {
		if o.Type == "PipeWire:Interface:Link" {
			existe[[2]int{o.Info.OutputPortID, o.Info.InputPortID}] = true
		}
	}

	discord := map[int]bool{}
	for _, s := range saidasApps {
		props := nos[s.no]
		if _, visto := discord[s.no]; !visto {
			discord[s.no] = ehDiscordPA(props)
		}
		for _, canal := range lado(s.canal) {
			destino, ok := entradaSala[canal]
			if !ok {
				continue
			}
			par := [2]int{s.id, destino}
			switch {
			case discord[s.no] && existe[par]:
				exec.Command("pw-link", "-d", strconv.Itoa(s.id), strconv.Itoa(destino)).Run()
			case !discord[s.no] && !naoEhApp(props) && !existe[par]:
				exec.Command("pw-link", strconv.Itoa(s.id), strconv.Itoa(destino)).Run()
			}
		}
	}

	// Ninguém deve tocar DIRETO na Sala (ela é muda; só recebe cópias). Se um app
	// foi parar nela (ex.: o WirePlumber lembrou de uma versão antiga deste
	// programa, ou escolhido à mão), volta para a saída padrão: assim você continua
	// ouvindo, e a cópia leva o som para a transmissão (menos o Discord).
	var entradas []entradaPA
	var sinks []struct {
		Index int
		Name  string
	}
	if pactlJSON(&entradas, "list", "sink-inputs") == nil && pactlJSON(&sinks, "list", "sinks") == nil {
		saidaPadrao, _ := exec.Command("pactl", "get-default-sink").Output()
		padrao := strings.TrimSpace(string(saidaPadrao))
		for _, s := range sinks {
			if s.Name != sinkSala || padrao == "" || padrao == sinkSala {
				continue
			}
			for _, e := range entradas {
				if e.Sink == s.Index && !naoEhApp(e.Properties) {
					exec.Command("pactl", "move-sink-input", strconv.Itoa(e.Index), padrao).Run()
				}
			}
		}
	}
}

// removerSalasAntigas descarrega uma "sala_de_tela" que tenha sobrado (ex.: o
// programa fechou de repente na transmissão anterior).
func removerSalasAntigas() {
	var modulos []struct {
		Index    int
		Name     string
		Argument string
	}
	if pactlJSON(&modulos, "list", "modules") != nil {
		return
	}
	for _, m := range modulos {
		if m.Name == "module-null-sink" && strings.Contains(m.Argument, "sink_name="+sinkSala) {
			exec.Command("pactl", "unload-module", strconv.Itoa(m.Index)).Run()
		}
	}
}

// iniciarAudioSala cria a "sala_de_tela" e mantém as cópias até fim fechar.
// Devolve false se não deu para montar (então transmitimos sem som).
func iniciarAudioSala(fim <-chan struct{}) bool {
	for _, p := range []string{"pactl", "pw-dump", "pw-link"} {
		if _, err := exec.LookPath(p); err != nil {
			return false
		}
	}
	removerSalasAntigas()
	saida, err := exec.Command("pactl", "load-module", "module-null-sink",
		"sink_name="+sinkSala, "rate=48000", "channels=2", "channel_map=front-left,front-right",
		"sink_properties='device.description=Sala-de-Tela priority.session=1 priority.driver=1'").Output()
	if err != nil {
		return false
	}
	modulo := strings.TrimSpace(string(saida))

	go func() {
		ajustarAudio()
		eventos := exec.Command("pactl", "subscribe")
		prepararFilho(eventos)
		linhas, _ := eventos.StdoutPipe()
		if eventos.Start() == nil {
			go func() {
				sc := bufio.NewScanner(linhas)
				for sc.Scan() {
					l := sc.Text()
					if strings.Contains(l, "sink-input") || strings.Contains(l, "'server'") {
						time.Sleep(150 * time.Millisecond) // as portas do app aparecem logo depois
						ajustarAudio()
					}
				}
			}()
		}
		// Reserva: confere de tempos em tempos (apps nativos do PipeWire nem sempre geram evento).
		tique := time.NewTicker(2 * time.Second)
		defer tique.Stop()
		for {
			select {
			case <-tique.C:
				ajustarAudio()
				continue
			case <-fim:
			}
			break
		}
		if eventos.Process != nil {
			eventos.Process.Kill()
			eventos.Wait()
		}
		// A "sala_de_tela" some e as cópias somem junto; os apps nem percebem.
		if err := exec.Command("pactl", "unload-module", modulo).Run(); err != nil {
			log.Printf("não consegui remover a saída de áudio da Sala: %v", err)
		}
	}()
	return true
}

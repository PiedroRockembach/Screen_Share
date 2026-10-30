package main

import (
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

func diretorioDados() string {
	if d := os.Getenv("LOCALAPPDATA"); d != "" {
		return filepath.Join(d, "SalaDeTela")
	}
	casa, _ := os.UserHomeDir()
	return filepath.Join(casa, "SalaDeTela")
}

func abrirNavegador(url string) {
	cmd := exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
	_ = cmd.Start()
	go cmd.Wait()
}

// prepararFilho esconde a janela preta dos programas de linha de comando.
func prepararFilho(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{HideWindow: true, CreationFlags: windows.CREATE_NO_WINDOW}
}

// Um "job" do Windows mata os filhos (MediaMTX, GStreamer, Tailscale Funnel)
// se o Sala de Tela fechar de qualquer jeito.
var job windows.Handle

func init() {
	h, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		log.Printf("job object: %v", err)
		return
	}
	info := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	info.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	_, err = windows.SetInformationJobObject(h, windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&info)), uint32(unsafe.Sizeof(info)))
	if err != nil {
		log.Printf("job object: %v", err)
		return
	}
	job = h
}

func vincularFilho(cmd *exec.Cmd) {
	if job == 0 || cmd.Process == nil {
		return
	}
	p, err := windows.OpenProcess(windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE, false, uint32(cmd.Process.Pid))
	if err != nil {
		return
	}
	defer windows.CloseHandle(p)
	_ = windows.AssignProcessToJobObject(job, p)
}

// comoAdministrador roda um instalador pedindo o "Sim" do UAC e espera terminar.
func comoAdministrador(exe string, args string) error {
	ps := fmt.Sprintf("$p = Start-Process -FilePath '%s' -ArgumentList '%s' -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
		strings.ReplaceAll(exe, "'", "''"), strings.ReplaceAll(args, "'", "''"))
	cmd := exec.Command("powershell", "-NoProfile", "-NonInteractive", "-Command", ps)
	prepararFilho(cmd)
	saida, err := cmd.CombinedOutput()
	if err != nil {
		if ee, ok := err.(*exec.ExitError); ok && (ee.ExitCode() == 0 || ee.ExitCode() == 3010) {
			return nil
		}
		return fmt.Errorf("instalador %s: %v %s (você clicou em Não no aviso do Windows?)", filepath.Base(exe), err, strings.TrimSpace(string(saida)))
	}
	return nil
}

// Processo é uma linha da lista de processos do Windows.
type Processo struct {
	PID, Pai uint32
	Nome     string
}

func listarProcessos() ([]Processo, error) {
	snap, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPPROCESS, 0)
	if err != nil {
		return nil, err
	}
	defer windows.CloseHandle(snap)
	var e windows.ProcessEntry32
	e.Size = uint32(unsafe.Sizeof(e))
	var lista []Processo
	for err = windows.Process32First(snap, &e); err == nil; err = windows.Process32Next(snap, &e) {
		lista = append(lista, Processo{PID: e.ProcessID, Pai: e.ParentProcessID, Nome: windows.UTF16ToString(e.ExeFile[:])})
	}
	return lista, nil
}

func versaoWindows() uint32 {
	return windows.RtlGetVersion().BuildNumber
}

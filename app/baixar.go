package main

import (
	"archive/tar"
	"archive/zip"
	"bufio"
	"compress/gzip"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// baixar salva url em destino, avisando o progresso a cada poucos segundos.
func (a *App) baixar(url, destino string) error {
	resp, err := http.Get(url)
	if err != nil {
		return fmt.Errorf("baixar %s: %w (sem internet?)", url, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("baixar %s: HTTP %d", url, resp.StatusCode)
	}
	if err := os.MkdirAll(filepath.Dir(destino), 0o755); err != nil {
		return err
	}
	tmp := destino + ".parcial"
	f, err := os.Create(tmp)
	if err != nil {
		return err
	}
	total := resp.ContentLength
	var feito int64
	ultimo := time.Now()
	buf := make([]byte, 256*1024)
	for {
		n, errLer := resp.Body.Read(buf)
		if n > 0 {
			if _, err := f.Write(buf[:n]); err != nil {
				f.Close()
				return err
			}
			feito += int64(n)
			if time.Since(ultimo) > 3*time.Second && total > 0 {
				a.registrar("baixando %s: %d%% de %d MB", filepath.Base(destino), feito*100/total, total>>20)
				ultimo = time.Now()
			}
		}
		if errLer == io.EOF {
			break
		}
		if errLer != nil {
			f.Close()
			return fmt.Errorf("baixar %s: %w", url, errLer)
		}
	}
	if err := f.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, destino)
}

func sha256Arquivo(caminho string) (string, error) {
	f, err := os.Open(caminho)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

// somaEsperada acha o hash de nomeArquivo num arquivo de checksums
// (linhas "hash  nome" ou "hash *nome"; com um só hash, aceita ele).
func somaEsperada(conteudo, nomeArquivo string) (string, error) {
	sc := bufio.NewScanner(strings.NewReader(conteudo))
	var unico string
	linhas := 0
	for sc.Scan() {
		campos := strings.Fields(sc.Text())
		if len(campos) == 0 {
			continue
		}
		linhas++
		unico = strings.ToLower(campos[0])
		if len(campos) >= 2 && strings.TrimPrefix(campos[1], "*") == nomeArquivo {
			return strings.ToLower(campos[0]), nil
		}
	}
	if linhas == 1 && len(unico) == 64 {
		return unico, nil
	}
	return "", fmt.Errorf("checksum de %s não encontrado", nomeArquivo)
}

func lerURL(url string) (string, error) {
	resp, err := http.Get(url)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("%s: HTTP %d", url, resp.StatusCode)
	}
	b, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	return string(b), err
}

// baixarVerificado baixa url, confere com o checksum publicado em urlSomas e devolve o caminho.
func (a *App) baixarVerificado(url, urlSomas, destino string) error {
	somas, err := lerURL(urlSomas)
	if err != nil {
		return err
	}
	esperado, err := somaEsperada(somas, filepath.Base(url))
	if err != nil {
		return err
	}
	if obtido, err := sha256Arquivo(destino); err == nil && obtido == esperado {
		return nil // já baixado
	}
	if err := a.baixar(url, destino); err != nil {
		return err
	}
	obtido, err := sha256Arquivo(destino)
	if err != nil {
		return err
	}
	if obtido != esperado {
		os.Remove(destino)
		return fmt.Errorf("o download de %s veio corrompido; tente de novo", filepath.Base(url))
	}
	return nil
}

// extrairDe copia um único arquivo (pelo nome) de dentro de um .tar.gz ou .zip.
func extrairDe(pacote, nome, destino string) error {
	var origem io.Reader
	if strings.HasSuffix(pacote, ".zip") {
		z, err := zip.OpenReader(pacote)
		if err != nil {
			return err
		}
		defer z.Close()
		for _, f := range z.File {
			if filepath.Base(f.Name) == nome {
				rc, err := f.Open()
				if err != nil {
					return err
				}
				defer rc.Close()
				origem = rc
				break
			}
		}
	} else {
		f, err := os.Open(pacote)
		if err != nil {
			return err
		}
		defer f.Close()
		gz, err := gzip.NewReader(f)
		if err != nil {
			return err
		}
		tr := tar.NewReader(gz)
		for {
			h, err := tr.Next()
			if err == io.EOF {
				break
			}
			if err != nil {
				return err
			}
			if filepath.Base(h.Name) == nome {
				origem = tr
				break
			}
		}
	}
	if origem == nil {
		return fmt.Errorf("%s não está dentro de %s", nome, filepath.Base(pacote))
	}
	tmp := destino + ".novo"
	out, err := os.OpenFile(tmp, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o755)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, origem); err != nil {
		out.Close()
		return err
	}
	if err := out.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, destino)
}

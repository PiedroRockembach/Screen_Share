package main

import "testing"

func TestFunnelPermitido(t *testing.T) {
	const dns = "piedro-desktop.tail856fca.ts.net"

	tests := []struct {
		nome string
		json string
		ok   bool
	}{
		{
			nome: "formato antigo",
			json: `{"AllowFunnel":{"piedro-desktop.tail856fca.ts.net:443":true}}`,
			ok:   true,
		},
		{
			nome: "formato foreground",
			json: `{"Foreground":{"node":{"AllowFunnel":{"piedro-desktop.tail856fca.ts.net:443":true}}}}`,
			ok:   true,
		},
		{
			nome: "outro endereco",
			json: `{"Foreground":{"node":{"AllowFunnel":{"outro.ts.net:443":true}}}}`,
			ok:   false,
		},
		{
			nome: "json invalido",
			json: `{`,
			ok:   false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.nome, func(t *testing.T) {
			if got := funnelPermitido([]byte(tt.json), dns); got != tt.ok {
				t.Fatalf("funnelPermitido() = %v, want %v", got, tt.ok)
			}
		})
	}
}

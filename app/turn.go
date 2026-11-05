package main

import (
	"errors"
	"fmt"
	"net"
	"strings"

	"github.com/pion/turn/v4"
)

// Retransmissão (TURN): o vídeo entre este PC e os convidados, nos dois
// sentidos. O TURN escuta TCP só em 127.0.0.1 e o Funnel do Tailscale o publica
// em https://<pc>.ts.net:8443 (TLS terminado pelo Tailscale). Para a rede é só
// HTTPS, então passa por roteadores, CGNAT e agregadores como o net-merge.
//
// Ele só entrega pacotes para portas altas (>= 1024) do próprio PC, no ipRelay:
// não serve de proxy para a rede local nem para a internet. O ipRelay é um IP
// real deste PC (o do Tailscale), não 127.0.0.1: com loopback o MediaMTX nunca
// fechava a conexão.

const (
	portaTURN        = 47805 // TCP, só 127.0.0.1
	portaTURNPublica = 8443  // porta pública do Funnel (TLS)
	usuarioTURN      = "sala"
	reinoTURN        = "sala-de-tela"
)

// connFiltrada descarta tudo que não seja de/para o próprio PC (portas altas).
type connFiltrada struct {
	net.PacketConn
	ip net.IP
}

func (c *connFiltrada) permitido(addr net.Addr) bool {
	u, ok := addr.(*net.UDPAddr)
	return ok && u.IP.Equal(c.ip) && u.Port >= 1024
}

func (c *connFiltrada) WriteTo(p []byte, addr net.Addr) (int, error) {
	if !c.permitido(addr) {
		return len(p), nil
	}
	return c.PacketConn.WriteTo(p, addr)
}

func (c *connFiltrada) ReadFrom(p []byte) (int, net.Addr, error) {
	for {
		n, addr, err := c.PacketConn.ReadFrom(p)
		if err != nil || c.permitido(addr) {
			return n, addr, err
		}
	}
}

type relaySoEstePC struct {
	turn.RelayAddressGeneratorStatic
}

func (r *relaySoEstePC) AllocatePacketConn(network string, porta int) (net.PacketConn, net.Addr, error) {
	c, addr, err := r.RelayAddressGeneratorStatic.AllocatePacketConn(network, porta)
	if err != nil {
		return nil, nil, err
	}
	return &connFiltrada{c, r.RelayAddress}, addr, nil
}

// ipParaRelay escolhe o IP deste PC por onde o TURN fala com o MediaMTX: o do
// Tailscale (sempre existe com a sala aberta) ou, em testes, o primeiro IPv4
// que não seja de loopback.
func ipParaRelay() (net.IP, error) {
	if binarioTailscale() != "" {
		if saida, err := comandoTS("ip", "-4").Output(); err == nil {
			if ip := net.ParseIP(strings.TrimSpace(strings.SplitN(string(saida), "\n", 2)[0])); ip != nil {
				return ip, nil
			}
		}
	}
	enderecos, _ := net.InterfaceAddrs()
	for _, e := range enderecos {
		if n, ok := e.(*net.IPNet); ok && n.IP.To4() != nil && !n.IP.IsLoopback() && !n.IP.IsLinkLocalUnicast() {
			return n.IP.To4(), nil
		}
	}
	return nil, errors.New("nenhum endereço de rede encontrado para a retransmissão")
}

func (a *App) iniciarTURN(ip net.IP) (*turn.Server, error) {
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", portaTURN))
	if err != nil {
		return nil, fmt.Errorf("iniciar a retransmissão (TURN): %w", err)
	}
	return turn.NewServer(turn.ServerConfig{
		Realm: reinoTURN,
		AuthHandler: func(usuario, reino string, _ net.Addr) ([]byte, bool) {
			if usuario != usuarioTURN {
				return nil, false
			}
			return turn.GenerateAuthKey(usuario, reino, a.config().SegredoTURN), true
		},
		ListenerConfigs: []turn.ListenerConfig{{
			Listener: ln,
			RelayAddressGenerator: &relaySoEstePC{turn.RelayAddressGeneratorStatic{
				RelayAddress: ip,
				Address:      ip.String(),
			}},
			PermissionHandler: func(_ net.Addr, destino net.IP) bool { return destino.Equal(ip) },
		}},
	})
}

// servidorICE é o que navegador e GStreamer usam para se conectar.
type servidorICE struct {
	URLs       []string `json:"urls"`
	Username   string   `json:"username,omitempty"`
	Credential string   `json:"credential,omitempty"`
}

func (a *App) servidoresICE(dns string) []servidorICE {
	return []servidorICE{{
		URLs:       []string{fmt.Sprintf("turns:%s:%d?transport=tcp", dns, portaTURNPublica)},
		Username:   usuarioTURN,
		Credential: a.config().SegredoTURN,
	}}
}

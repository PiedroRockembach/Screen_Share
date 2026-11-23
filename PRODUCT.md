# Sala de Tela

## O que é
Substituto gratuito do "Go Live" do Discord: um grupo de amigos transmite a tela entre si em 1080p60, com o som do PC e nunca com a voz da chamada do Discord. Cada um assiste dentro do próprio app.

## Quem usa e quando
Amigos leigos em tecnologia, em casa, à noite, normalmente já numa chamada do Discord, jogando ou assistindo algo juntos. Precisa funcionar como "instalou, usou": nenhuma configuração, nenhum termo técnico na tela.

## Como funciona
- Qualquer um cria uma sala e recebe um convite (`sala1-…`) para mandar no chat. O convite continua valendo nas próximas vezes.
- Quem recebe cola o convite e entra. Dentro da sala, qualquer um pode transmitir a própria tela e assistir à de quem está ao vivo.
- A conexão é direta entre os PCs (túnel UDP próprio, rede de casa e IPv6), sem serviço de terceiros.

## Restrições fixas
- 100% gratuito; sem Tailscale nem serviços de terceiros na rede.
- Saída fixa em 1080p60, nada acima. A taxa vai até 20 Mbps e cai sozinha em rede fraca (até 2 Mbps); a resolução não muda.
- Se não der para tirar o Discord do áudio com segurança, transmite sem som.
- Quando não há caminho direto entre dois amigos, avisa e não conecta.

## Plataforma
App desktop para Linux e Windows (Rust, Slint, GStreamer). Instalador no Windows; no Linux, por enquanto, roda pelo `cargo`.

## Linguagem
Português do Brasil na interface, em caixa de frase, sem jargão de rede ("convite", "sala", "ao vivo", nunca "IP", "porta", "SRT").

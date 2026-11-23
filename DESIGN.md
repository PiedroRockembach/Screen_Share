# Design da Sala de Tela

Mundo: a sala de casa à noite, com a TV ligada. Modo Operate: a estrutura é a de um app desktop comum (painel à esquerda, vídeo à direita, controles padrão); o mundo entra só por paleta, fonte, densidade e um gesto. Implementação em `desktop/ui/app.slint` (global `Theme`).

## Cena
Amigos em casa, à noite, luz baixa, numa chamada do Discord. Por isso o escuro: em volta de vídeo, tela clara ofusca.

## Cor
Estratégia contida com casca assumida: o painel tem cor própria (petróleo) e o palco tem a sua (penumbra quente). Todos os pares de texto passam de 4,5:1; bordas de controle passam de 3:1.

| Token | Hex | Papel |
|---|---|---|
| `wall` | `#15110e` | palco do vídeo, parede na penumbra |
| `wall-raised` | `#211b18` | barra de controles sob o vídeo, chips |
| `wall-line` | `#332d28` | hover de botão sobre a parede |
| `wall-text` | `#efebe6` | texto sobre a parede (15,8:1) |
| `wall-text-2` | `#c2b7ae` | texto secundário sobre a parede (9,5:1) |
| `velvet` | `#14292e` | painel lateral, o sofá |
| `velvet-raised` | `#1e373e` | campos, botão secundário, hover de linha |
| `velvet-hover` | `#26424a` | hover do secundário, linha selecionada |
| `velvet-line` | `#2f4950` | botão secundário pressionado |
| `velvet-control` | `#648790` | borda de controle (3,9:1) |
| `text` / `text-2` / `text-3` | `#ebf3f4` / `#bbcfd3` / `#95adb2` | três níveis de texto no painel (13,5 / 9,4 / 6,4:1) |
| `lamp` | `#f6ae31` | ação primária, foco de campo, alternância ligada |
| `lamp-hover` / `lamp-press` | `#fbbc51` / `#f0960f` | estados da primária |
| `lamp-ink` | `#301b08` | texto sobre âmbar (8,6:1) |
| `live` | `#e93b35` | ponto "ao vivo"; só significa "no ar" |
| `live-text` | `#fa9189` | texto de erro e de alerta (6,9:1) |
| `danger` / `danger-hover` | `#ca2621` / `#b31f1b` | só o botão dentro da confirmação de fechar a sala |

## Tipo
Atkinson Hyperlegible Next (OFL, Braille Institute), 400 e 700, escolhida pela legibilidade para amigos leigos. Atkinson Hyperlegible Mono só no código do convite, que é um código para conferir. Escala: 12 (detalhe técnico, dicas), 14 (corpo e botões), 16 (texto do palco, "Ao vivo como"), 20 (marca e título do palco). Arquivos em `desktop/ui/fonts/`.

## Espaço e forma
Escala 4, 8, 12, 16, 24, 32, 48. Painel de 320 px, margem 24, 32 entre grupos e 12 dentro deles. Raio 6 px em tudo que é clicável, sem sombra: a elevação vem do tom. Botões com 40 px (primário, secundário, perigo) e 36 px (discreto). Ícones Lucide (ISC) com traço 2, em 16 px, sempre com texto, em `desktop/ui/icons/`.

## Componentes
- `Button`: `primary` (âmbar, uma por tela), `secondary` (veludo elevado com borda), `ghost` e `ghost-on-wall` (sem fundo), `danger` (só na confirmação). `busy` mantém as cores e troca o rótulo.
- `TextField`: fundo elevado, borda de controle, foco com borda âmbar de 2 px, seleção em âmbar translúcido.
- `Toggle`: trilho 36×20, ligado em âmbar; rótulo e dica abaixo.
- `LiveRow`: ponto vermelho, nome, etiqueta "você" ou "assistindo".
- `Note`: linha de status; com `error`, ícone de alerta e `live-text`.
- Foco de teclado: borda de 2 px em `text` nos botões, linhas e alternância.

## Gesto: luz ambiente
O palco é um gradiente radial da cor `glow` para `wall`. `glow` é a cor média do quadro, calculada no Rust (`ambient_light` em `desktop/src/main.rs`) cerca de 10 vezes por segundo, com saturação até 60% e luminosidade entre 8% e 24%, e anima em 400 ms com ease-out exponencial. Aparece nas faixas em volta do vídeo, como a TV iluminando a parede. Sem vídeo, o palco volta a `wall`.

## Movimento
Só estado: cor de fundo dos botões em 150 ms, a bolinha da alternância em 150 ms e a luz ambiente em 400 ms, sempre com `cubic-bezier(0.23, 1, 0.32, 1)`. "Convite copiado" e a dica "Esc para sair da tela cheia" trocam na hora e somem sozinhas (2 s e 3 s).

## Texto
Caixa de frase, verbo no botão, sem jargão de rede, erro dizendo o que houve e o que fazer. Atalhos: F11 (tela cheia), Esc (sair dela), M (som).

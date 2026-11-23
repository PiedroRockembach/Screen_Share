# Surface brief: janela do app desktop (desktop/ui/app.slint)

Modo: Operate. Tarefa: criar ou entrar numa sala, transmitir a própria tela, assistir à de um amigo. Estados que importam: sem sala, abrindo/procurando, anfitrião, convidado (conectado ou sem resposta), transmitindo, assistindo, tela cheia, erros de rede e de captura.

## Direction contract

THESIS: a janela é a sala de casa à noite com a TV ligada. Recusa o clone escuro de Discord/Twitch (cinza neutro com um acento): o ambiente tem cor própria e reage ao que se assiste.

OWN-WORLD: paredes na penumbra em marrom quente (#15110e) em volta do vídeo; painel lateral em petróleo profundo, o veludo do sofá (#14292e); ação primária em âmbar de abajur (#f6ae31) com tinta marrom; vermelho (#e93b35) só para "no ar". Atkinson Hyperlegible Next 400/700, Mono só no código do convite. Raio 6 px, sem sombras, ícones Lucide de traço 2.

STORY: quem abre entende em segundos que cria uma sala ou cola um convite; dentro, vê quem está ao vivo, transmite com um botão e sabe sempre se está no ar.

FIRST VIEWPORT: painel de 320 px à esquerda (marca, "Criar sala" em âmbar, campo do convite); à direita o palco do vídeo com o texto que ensina o fluxo. Na sala: convite e cópia no topo, transmissão no meio, "Ao vivo agora" embaixo, sair no pé; barra de controles sob o vídeo.

FORM: gesto característico, luz ambiente: o fundo em volta do vídeo acende com a cor média da imagem (escurecida), em transição de 400 ms, como a TV iluminando a parede. Primeira da lista ordenada. seed: none (unavailable)

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance

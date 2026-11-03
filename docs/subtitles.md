# Modo legenda — 30/10/2026

## Abrir e ajustar

Use “Legenda” na notch expandida, na janela de letras ou no menu da bandeja. A legenda abre em modo de ajuste. Arraste a área livre para mover e use as bordas laterais para ajustar a largura. A− e A+ ajustam uma fonte própria, inicialmente 36 DIP, entre 18 e 96. O texto tem quebra de linha e alinhamento central.

Escolha Original, Tradução ou Ambos. Em Ambos, a tradução aparece abaixo com fonte menor. A falta de tradução deixa esse texto vazio, sem substituí-lo por um verso diferente. “Fundo” alterna uma faixa escura semitransparente opcional.

Os controles −0,1 s e +0,1 s alteram a compensação entre −10 e +10 segundos. Valores positivos adiantam os versos; negativos atrasam. A compensação afeta somente a legenda. Tamanho, posição, fonte, idioma, fundo, compensação e escolha de atalho ficam salvos em `Settings::subtitles`, com valores padrão compatíveis com configurações antigas.

## Travar e recuperar os controles

“Travar” esconde a moldura e a barra de ajustes. A janela é uma superfície por pixel: o fundo fica transparente e os glifos continuam opacos com contorno preto e sombra. Os estilos WS_EX_TRANSPARENT e WS_EX_NOACTIVATE deixam cliques passarem para a aplicação abaixo sem reativar a legenda. Ao destravar, esses estilos são removidos.

Ctrl+Alt+L alterna ajustar/travar, inclusive quando a janela não aceita cliques. “Atalho” permite escolher Ctrl+Alt+L, Ctrl+Alt+S ou Ctrl+Shift+L. Se o registro do atalho falhar por conflito, o app apresenta um aviso e o menu da bandeja continua disponível: Abrir/ajustar legenda, Travar/destravar legenda e Ocultar legenda.

O modo travado não é restaurado automaticamente ao abrir o aplicativo: a legenda abre por solicitação do usuário, com os controles disponíveis. Fechar a legenda não encerra o Nivumi nem a reprodução.

## Reprodução e monitores

A legenda usa os versos e a posição atual do mesmo serviço de mídia, sem criar novas buscas. Pausar mantém o verso; seek seleciona novamente o texto. Antes do primeiro timestamp e nos intervalos vazios/instrumentais, o texto é removido. A atualização usa o heartbeat existente de 250 ms.

A superfície é limitada à área útil do monitor, sem reticências. Versos excepcionalmente extensos podem exigir mais largura ou fonte menor. Mudanças de DPI refazem a renderização; posições fora dos monitores conectados são recuperadas na área útil mais próxima. A janela permanece acima das janelas normais, independentemente de a notch estar recolhida ou escondida.

## Verificação

52 testes de biblioteca/binário passaram. Os novos testes cobrem limites temporais, seek para trás, compensação, tradução ausente, intervalos instrumentais, normalização e serialização de preferências.

Um teste Win32 oculto renderiza a camada real com Direct2D/DirectWrite em um bitmap premultiplicado. Ele confirma pixels de fundo com alpha zero, pixels de texto com alpha 255, remoção do verso no intervalo instrumental, ativação/remoção dos estilos de passagem de cliques e ausência de uma janela visível durante o teste. Não há automação de entrada em outros aplicativos nesse teste.

Formatação e Clippy de todos os targets passaram. Arraste manual, cliques em uma aplicação subjacente real, atalhos globais em conflito, troca física de monitor e a aparência sobre vídeos/jogos não foram exercitados manualmente nesta revisão. A presença de uma janela topmost não comprova exibição sobre jogos em tela cheia exclusiva.


## Ajustes discretos por menu de contexto

A barra de botões foi removida. No modo de ajuste aparece somente uma moldura fina; o fundo de captura é quase transparente quando a faixa escura está desligada. Arraste normalmente para mover ou use as bordas laterais para ajustar a largura. Clique com o botão direito para abrir o menu: tamanho do texto, original/tradução/ambos, compensação e redefinição de tempo, atalho, faixa escura, travar e ocultar. Shift+F10, a tecla de menu, Enter ou Espaço também abrem as configurações com foco na legenda.

Quando estiver travada, a passagem de cliques continua ativa, portanto o botão direito vai para a aplicação abaixo. Use o atalho configurado (Ctrl+Alt+L por padrão) ou a bandeja para destravar e então abrir o menu. Sem um verso disponível, o modo de ajuste mostra uma dica curta sobre o botão direito.


## Atalhos individuais de sincronização e troca de verso

Em botão direito → Atalhos de sincronização, escolha Atrasar, Adiantar ou Zerar para alterar sua combinação. A legenda solicita Ctrl ou Alt mais uma tecla; Shift é opcional. Escape ou mudar o foco cancela a captura. Os atalhos da legenda são suspensos durante essa captura e registrados novamente ao terminar. Uma combinação repetida substitui o uso anterior entre as três ações. Cada ação também pode ser desativada pelo menu.

Valores iniciais: Ctrl+Alt+tecla − para atrasar 0,1 s, Ctrl+Alt+tecla +/= para adiantar 0,1 s, Ctrl+Alt+0 para zerar. A combinação reconhece a tecla física virtual configurada: em teclados que exigem Shift para digitar +, use a tecla +/= sem Shift ou grave a combinação desejada. Os atalhos atuam também com a legenda travada; se ela ainda não estiver aberta, ajustam a compensação salva. A faixa continua limitada a ±10 s. Conflitos de registro geram aviso e os ajustes pelo menu continuam disponíveis.

Novas frases entram em 180 ms, com opacidade do texto indo de 75% a 100% e deslocamento vertical de 3 DIP até a posição final, usando uma curva de desaceleração. O texto anterior é substituído imediatamente, sem sobreposição. A moldura e o fundo não são animados. O timer existe somente durante a transição. A opção de movimento reduzido do aplicativo desliga a transição e interrompe uma animação em andamento.

55 testes passaram, incluindo configurações antigas, persistência/validação de combinações, progressão e término da curva de animação e os testes nativos de transparência existentes. Clippy e formatação passaram. A captura manual das teclas, conflitos reais com outros aplicativos e a fluidez visual não foram exercitados manualmente nesta revisão.


## Indicador animado de sincronização (HUD)

Ao usar os atalhos de sincronização (Atrasar, Adiantar, Zerar) ou os comandos correspondentes no menu de contexto, um indicador flutuante em formato de pílula (HUD) surge sobre a legenda para fornecer feedback visual imediato:

- **Identificação clara**: mostra o valor em segundos e o estado atual (`Adiantado: +0,1 s`, `Atrasado: -0,1 s` ou `Sincronizado: 0,0 s`).
- **Medidor visual em tempo real**: uma barra com marcação central (0,0 s) exibe graficamente o deslocamento temporal entre −5 e +5 segundos (ciano para adiantado, âmbar para atrasado e marcador verde para sincronizado).
- **Animação fluida e transitória**: entra suavemente em 160 ms com curva cúbica de desaceleração e leve deslocamento vertical, permanece legível e desvanece suavemente ao final de 1,5 segundo. O HUD só aparece quando há alteração.
- **Ajustes sucessivos**: toques repetidos nos atalhos atualizam o valor e a barra de forma contínua, reiniciando o tempo de exibição para permitir ajustes finos sem interrupção.
- **Movimento reduzido**: quando ativado nas configurações, o deslocamento é desativado e o indicador permanece fixo até desaparecer.
- **Legenda fechada**: se a legenda não estiver aberta ao acionar os atalhos globais, um aviso transitório na notch informa o novo estado de compensação.

## Sincronização compartilhada por música

Os atalhos e o menu de compensação agora ajustam também a letra do painel e do leitor separado. O valor é salvo por título e artista, sem afetar as outras músicas. Consulte [lyrics-sync.md](lyrics-sync.md) para comportamento e testes atuais.

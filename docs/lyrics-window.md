# Letras redimensionáveis e janela separada — 30/10/2026

## Alinhamento da barra superior

As dicas flutuantes por hover foram removidas de toda a notch, incluindo os balões de ação e percentual do volume. Os nomes dos controles e o valor de volume continuam disponíveis para acessibilidade. O realce ao passar o mouse e a barra de volume permanecem funcionais; o temporizador usado exclusivamente pelas dicas também foi removido.

No mini-player ao lado das letras, os cinco controles (letras, anterior, play/pausa, próxima e volume) têm centros igualmente espaçados em 38 px. Letras e volume usam alvos iguais de 36 × 38 px e seus ícones compartilham o centro vertical dos controles de reprodução. A barra de volume aparece abaixo da fileira ao interagir com ela, sem reservar um espaço horizontal entre próxima e volume. O teste de geometria cobre simetria e separação entre a barra e os botões; os testes existentes cobrem arraste, mapeamento de volume e continuidade de hover. Não houve nova conferência visual com reprodução nesta revisão.

Na notch expandida, abrir em janela, CC, opções e fechar compartilham o mesmo centro vertical (22,5 px) e alvos de 32 × 32 px, lado a lado sem sobreposição. A posição parte da borda direita, mantendo o espaçamento ao redimensionar. A área do seletor de modo e do indicador offline respeita o início desse grupo. Um teste de geometria verifica alinhamento e limites em larguras de 520 a 1800 px. Esta revisão não teve nova conferência visual em reprodução.

## Uso

Na página de letras, arraste as laterais ou a borda inferior da notch expandida para mudar o espaço de leitura. O mini-player conserva suas medidas; a coluna de versos aproveita a área adicional. O cursor de redimensionamento e a marca de grip no canto inferior direito indicam as regiões de arraste (posicionada de forma limpa e sem sobrepor as bolinhas de navegação centralizadas). O tamanho é limitado ao monitor selecionado e fica salvo nas configurações.

Os controles A− e A+ ajustam a fonte. Na notch, Ctrl+mais e Ctrl+menos também funcionam. Os versos e a tradução quebram em linhas, sem reticências. O rodapé redundante “Sync + Tradução PT-BR” foi removido. Se um verso inteiro exceder a área disponível, aumente a altura/largura, ajuste a fonte ou abra a janela de leitura.

O botão “Janela ↗” abre uma janela independente. A barra de título permite mover, maximizar e fechar; as bordas permitem redimensionar. Os botões Fixar/Desafixar alternam a permanência acima de outras janelas. Há modos Sync, Sync+Trad, Letras e Tradução. A fonte da janela tem seu próprio ajuste.

O leitor acompanha o verso atual. A roda do mouse, a barra de rolagem e as teclas de navegação pausam esse acompanhamento. “Voltar ao verso atual” o retoma. Fechar o leitor mantém o player e a notch; reabrir usa a mesma janela e o conteúdo atualizado. Tamanho, posição, fonte, modo e preferência de fixação são salvos. Uma posição lembrada fora dos monitores disponíveis é trazida para a área útil do monitor mais próximo.

As duas superfícies usam os versos e o índice de reprodução do mesmo serviço de mídia. Abrir a janela não cria outro worker de busca nem controla a reprodução separadamente. A notch recolhida continua mostrando o verso atual quando a página de letras está ativa.

## Implementação e compatibilidade

- `lyrics_view.rs`: conteúdo compartilhado, seleção de modos, preferências com valores padrão e recuperação de geometria fora da tela.
- `lyrics_window.rs`: janela Win32 independente com RichEdit Unicode somente leitura, quebra de linha nativa, rolagem, destaque do verso e tradução, teclado e controles de janela.
- `render.rs`: medição e desenho de parágrafos com DirectWrite para a notch.
- `main.rs`: arraste das bordas, superfície dinâmica preservando a região nativa de entrada, controles de fonte, abertura/atualização do leitor e persistência.
- `Settings::lyrics` usa serde com valores padrão. Configurações antigas continuam carregando; não há mudança do schema nem migração dos dados do Nusa original.

O posicionamento usa coordenadas físicas para a janela normal e medidas lógicas para a notch. O leitor aceita a mudança de DPI recomendada pelo Windows e reaplica o layout dos controles. A janela de letras não é escondida junto da notch durante um aplicativo em tela cheia; a preferência de fixação do leitor é independente.

## Verificação

49 testes unitários passaram com `cargo test --locked --lib --bins --quiet`. Há cobertura de configurações antigas, persistência das preferências, limites de fonte/geometria, monitores com coordenadas negativas ou desconectados, seleção de versos e preservação das demais funcionalidades. Formatação e Clippy em todos os targets passaram.

A prévia isolada `examples/lyrics_reader_preview.rs` usa texto sintético, sem ler ou modificar o estado do usuário. A captura e a árvore de acessibilidade confirmaram o leitor, o texto Unicode, a quebra de linhas longas, o destaque do verso, a tradução e os controles acessíveis. As primeiras capturas estavam encobertas por outra janela; a captura após ativar a prévia confirmou a identidade e o texto sintético corretos.

O usuário interrompeu o Computer Use com Escape ao iniciar o teste do botão de fonte. Não foram feitos outros comandos de interface após essa interrupção. Arraste, redimensionamento, fixação, restauração em múltiplos monitores e o ciclo completo com o player real não foram confirmados manualmente nesta revisão. A prévia encerra por conta própria após dez minutos ou quando fechada.

O teste genérico `cargo test` tentou reconstruir a prévia enquanto seu executável estava aberto e encontrou o bloqueio de arquivo do Windows. Os testes de biblioteca/binário foram então executados com sucesso; Clippy verificou também os exemplos. O release usa outro diretório e não substitui o aplicativo que já está aberto.


## Aparência e transparência

O leitor usa uma superfície escura uniforme na barra de título, nos controles e no texto. Os botões têm desenho arredondado, contraste de seleção e fonte Segoe UI ajustada ao DPI. A janela usa opacidade de 235/255 (aproximadamente 92%): a transparência é discreta e afeta a janela inteira, inclusive o texto. Não é um efeito de desfoque/acrílico. Cantos arredondados e cores de moldura/título são solicitados ao DWM quando suportados pelo Windows; versões ou composições incompatíveis podem conservar a moldura padrão.

A alteração de aparência passou pelos 49 testes de biblioteca/binário e pelo Clippy. A aparência desta revisão não foi conferida por uma nova captura em execução.

## Simplificação das telas e menus escuros

A notch tem uma barra única: modo de exibição, abrir em janela, CC (legenda) e opções. Fonte, acompanhamento e recarga ficam no menu de opções, que também abre com o botão direito na página expandida de letras. A área de leitura começa mais acima, aproveitando o espaço liberado pelos controles removidos.

O leitor separado usa uma barra única com modo, CC e opções. O botão de retorno ao verso aparece quando o acompanhamento é pausado por rolagem manual. Os ajustes de fonte, fixação e os quatro modos continuam no menu de opções; clicar com o botão direito no texto também abre esse menu. A barra horizontal foi explicitamente ocultada, mantendo quebra de linha, seleção e cópia pelo teclado.

Os menus de letras, legenda e bandeja usam o mesmo desenho escuro, fonte Segoe UI ajustada ao DPI, itens com espaçamento maior, seleção arredondada e indicadores de escolha. A navegação nativa, submenus e fechamento pelo teclado são preservados. A borda externa e a sombra do popup continuam sob controle do Windows.

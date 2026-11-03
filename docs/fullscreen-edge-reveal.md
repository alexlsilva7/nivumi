# Ocultação e acesso pela borda — 30/10/2026

A opção de ocultação passa a incluir janelas maximizadas, além de tela cheia. O detector anterior excluía explicitamente janelas com `IsZoomed` e `WS_CAPTION`, como o Chrome maximizado, mesmo com a opção ativada. O novo comportamento vale para os aplicativos no mesmo monitor do Nusa; desktop, barra de tarefas, janelas minimizadas e janelas ocultas por outro desktop virtual continuam excluídos.

## Acesso ao painel

- No estado oculto, permanece uma linha branca de 72 × 6 DIP, com fundo escuro para contraste. A região de acesso mede 96 × 10 DIP e encosta na borda superior física do monitor, alinhada à coluna esquerda, central ou direita escolhida para o Nusa.
- Passar o mouse na região revela e expande o painel. Clique, controle acessível “Mostrar Nusa” e o atalho existente também permitem abrir.
- O painel permanece disponível enquanto o cursor está sobre ele ou durante arraste, redimensionamento, ajuste de volume e diálogos. Ao sair, aguarda 600 ms antes de voltar à linha; a verificação periódica pode acrescentar até cerca de 250 ms.
- O hover revela sem ativar a janela. Se o usuário clicar no painel, o detector mantém referência à janela externa coberta. Ao recolher, devolve o foco a essa janela somente se o próprio Nusa ainda detiver o foco.
- Quando a janela externa deixa de estar maximizada/tela cheia, volta o comportamento normal do Nivumi. Desativar a opção também encerra esse modo. A opção foi renomeada para “Ocultar em tela cheia ou janela maximizada”, mantendo o campo persistido existente.

O estado oculto desenha apenas a linha e substitui a região nativa de entrada e os controles de acessibilidade; a área transparente restante não intercepta cliques. A revelação reaproveita a animação existente e respeita movimento reduzido. Não há novo timer de animação contínua para a linha.

## Verificação

- `cargo fmt --check`: passou na cópia isolada desta tarefa.
- `cargo test --locked --quiet`: 110 testes passaram (88 da biblioteca e 22 do binário).
- `cargo clippy --locked --all-targets -- -D warnings`: passou.
- Os casos novos cobrem Chrome maximizado com barra de tarefas visível, Chrome em janela normal, monitores com coordenadas negativas, janela maximizada fora do monitor alvo, revelação pela borda, espera antes de ocultar, reentrada do cursor, interação durante arraste, saída do modo coberto, limites da região clicável e abertura explícita com movimento reduzido.

A tentativa de inspeção visual foi interrompida pelo Computer Use por não conseguir determinar a URL atual do Chrome com confiança. A prévia isolada foi compilada, mas não foi aberta após esse bloqueio. Portanto, o fluxo real no Chrome e o desenho da linha ainda precisam de conferência visual. Jogos em tela cheia exclusiva podem impedir overlays compostos pelo Windows; não houve teste em jogos ou múltiplos monitores físicos. As verificações de geometria não garantem visibilidade sobre todos os modos gráficos exclusivos.

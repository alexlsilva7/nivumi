# Navegação da notch — 30/10/2026

A roda comum troca de página em qualquer área da notch, inclusive sobre letras, Shelf e tarefas. A ordem circular é Música → Pomodoro → Hidratação → Shelf → Tarefas → Calendário. Rolar para cima percorre a ordem inversa. Shift + roda preserva a rolagem do conteúdo nas páginas de letras, Shelf e tarefas.

Ao passar o mouse sobre os indicadores inferiores, eles se transformam numa barra animada com botões Música, Pomodoro, Água, Shelf, Tarefas e Configurações. A área de hover cobre toda a barra, incluindo os espaços entre botões. O modo de movimento reduzido troca diretamente. A região nativa da janela inclui a barra enquanto visível, preservando os cliques nas janelas atrás das áreas vazias.

Verificação: `cargo test --locked` (48 testes), `cargo clippy --locked --all-targets -- -D warnings` e formatação passaram. Esses testes validam a suíte existente; a interação de roda/hover exige conferência na janela.

## Letras como detalhe de Música

Letras abre pelo ícone de letras no player de Música. A navegação lateral (setas esquerda/direita), a roda vertical e a roda horizontal pulam Letras nos dois sentidos. Ao sair de Letras por esses atalhos, a navegação continua a partir de Música: anterior vai para Calendário e próximo vai para Pomodoro. Shift/Alt + roda continua rolando os versos.

O seletor com título fixo e menu de todas as seções foi removido, restaurando o cabeçalho anterior e mantendo a barra animada inferior. As alterações posteriores de hidratação foram preservadas.

Verificação desta revisão: 119 testes de biblioteca/binário, formatação e Clippy passaram. O teste de navegação cobre explicitamente os destinos nos dois sentidos, incluindo saída de Letras e fechamento do ciclo. A interação visual em execução não foi conferida nesta revisão.

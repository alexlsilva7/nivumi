# Formulário de configurações

## Aparência inspirada na notch

A janela usa uma paleta própria quase preta com azul, fundo com gradiente discreto, navegação lateral arredondada, seleção em pílula, cartões com bordas suaves e botões de ação em azul. Os campos editáveis têm contorno de foco e os seletores mantêm o comportamento nativo com desenho fechado personalizado. A moldura solicita cantos arredondados e cores escuras ao DWM quando suportados. Não há transparência nem desfoque; a legibilidade do formulário é preservada.

Os controles Win32 e a navegação por teclado permanecem nativos. A prévia isolada confirmou a paleta, os cartões e a navegação na página Geral. A inspeção foi interrompida por Escape antes de conferir outras páginas; o acabamento final dos seletores e a matriz de DPI não foram confirmados visualmente nesta revisão. Nenhuma preferência pessoal foi salva pela prévia.

Os campos de início e fim dos lembretes de água agora usam listas de horários em intervalos de uma hora. O fim pode ser definido até 24:00; um erro de intervalo aparece junto ao campo e mantém a janela aberta para correção. Campos numéricos inteiros aceitam apenas dígitos, e erros de faixa aparecem abaixo do respectivo campo com foco direcionado para ele.

A verificação automática desta mudança é `cargo check` e `cargo clippy --all-targets -- -D warnings`. Ainda é necessário conferir manualmente as listas de horário e os erros no Windows, incluindo teclado e leitor de tela.

# Formulário de tarefas

O editor usa seletores nativos de data e horário local. “Hoje” e “Amanhã” alteram a data e ativam o lembrete, preservando o horário escolhido. “Sem lembrete” desativa os seletores e salva a tarefa sem data. A paleta contém oito cores identificadas por nome e amostra; “Avançado: cor personalizada” revela o hexadecimal. Cores personalizadas de tarefas existentes são preservadas.

Ao salvar, erros aparecem abaixo do campo correspondente e o primeiro campo inválido recebe foco, sem fechar o formulário nem perder valores. Datas locais inexistentes ou ambíguas são rejeitadas. Cancelar mantém a tarefa original.

Verificação automatizada: `cargo test --lib` (94 testes aprovados, incluindo validação de cor e data inválida) e `cargo clippy --all-targets -- -D warnings` aprovados. Inspeção visual interativa, leitor de tela e escalas de DPI não foram exercitados nesta entrega.

Roteiro manual pendente:

- Criar tarefa sem lembrete; salvar e reabrir.
- Usar Hoje/Amanhã, alterar horário e conferir a persistência.
- Escolher uma data pelo calendário e uma cor pela paleta, usando mouse e teclado.
- Escolher cor personalizada, informar valor inválido e conferir erro junto ao campo, foco e retenção do título/data.
- Salvar com título vazio; corrigir e salvar novamente.
- Editar tarefa com cor personalizada e lembrete existente; cancelar e conferir que nada mudou.
- Conferir português/inglês, Tab/Shift+Tab, Escape, Enter, leitor de tela e DPI 100/150/200%.

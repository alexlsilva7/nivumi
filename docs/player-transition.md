# Player compartilhado entre Música e Letras — 30/10/2026

As duas telas agora desenham o mesmo player. Isso mantém capa, título, artista e reprodução visíveis ao navegar entre Música e Letras, sem substituir todo o conteúdo de uma vez. Somente a geometria que muda acompanha uma mola amortecida: largura do player, texto e progresso, posição dos controles e volume. O cabeçalho da música sai por opacidade e o painel de letras aparece depois que os controles liberam sua área. O fundo e a navegação continuam usando a animação existente.

O volume permanece disponível nas letras, em uma linha abaixo da reprodução, com barra mais larga. Clique e arraste, roda do mouse, ajuste fino com Shift/Ctrl, mudo e percentual usam o mesmo código nas duas telas. Desenho, áreas clicáveis e cálculo do volume usam a geometria animada atual, inclusive durante a troca e ao redimensionar. A roda sobre o volume não muda de página.

A navegação por botão, bolinhas, roda ou setas do teclado usa o mesmo caminho. Trocar novamente durante a animação preserva a posição e a velocidade da mola. Ao vir de outra funcionalidade ou abrir a notch recolhida, o player começa no layout de destino. Movimento reduzido aplica diretamente as dimensões e o layout finais.

## Verificação

- `cargo fmt --check`: passou.
- `cargo test --locked --quiet`: 101 testes passaram na cópia isolada (84 da biblioteca e 17 do binário).
- `cargo clippy --locked --all-targets -- -D warnings`: passou.
- Após aplicar a correção ao workspace com as demais alterações em andamento, os 102 testes e o Clippy também passaram. A formatação global encontrou diferenças em trechos de navegação/prateleira dessas outras alterações; somente a cópia isolada desta tarefa foi formatada.
- Os testes novos verificam limites e mapeamento do volume durante a transição, área livre do leitor e da navegação, dimensões mínimas, hit testing após redimensionamento, ocultação do controle sem dispositivo/fora das telas de mídia e navegação com movimento reduzido.
- Prévia nativa isolada com perfil temporário, em escala de 150%: telas Música e Letras observadas com mídia real; volume visível abaixo da reprodução; roda 100% → 95% → 100% sem mudar de tela; arraste 100% → 50% → 100%; retorno para Música conferido. O nível original foi restaurado.

O usuário interrompeu o Computer Use com Esc antes da conferência visual no tamanho mínimo. Não houve medição de frame pacing nem teste visual em todos os DPI. A prévia usa identidade de instância e perfil próprios; o executável em uso e as preferências do usuário não foram substituídos. O controle de mudo foi reaproveitado, mas não foi acionado durante essa conferência visual.

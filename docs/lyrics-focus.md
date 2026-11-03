As preferências também estão em **Configurações → Leitura de letras**: escolha a animação e se deseja tradução em todos os versos, depois clique em Salvar alterações. Cancelar mantém as preferências anteriores. O menu ⋯ continua como atalho para as mesmas opções.

# Foco e tradução dos versos

O modo Deslizar mede somente o conteúdo visível, com a fonte e o peso realmente usados por cada verso. Traduções ocultas, ausentes ou vazias não reservam uma linha em branco. O espaço normal entre versos é preservado. Há testes de regressão para tradução longa oculta e tradução vazia; a rolagem continua usando os deslocamentos acumulados dessas mesmas alturas. Não houve nova conferência visual em reprodução nesta correção.

Na página de letras, abra **⋯ → Animação do verso na notch**. Opções:

- **Sem animação:** troca imediata.
- **Dissolver:** entrada suave do verso e de sua tradução.
- **Deslizar:** rolagem suave da lista até o verso atual, com destaque gradual do verso. A rolagem considera as alturas reais das linhas e traduções; alterações rápidas de foco continuam a partir da posição exibida.
- **Realce suave:** a cor do verso passa gradualmente para o destaque.

As transições duram 280 ms, são substituídas quando o foco muda novamente e respeitam **Reduzir animações**. O padrão é Dissolver. A animação se aplica à notch; a janela de leitura mantém o acompanhamento nativo.

No mesmo menu, **Tradução em todos os versos** permite mostrar as traduções disponíveis nos versos anteriores e seguintes, além do atual. Desmarcar limita a tradução ao verso em foco. O padrão passa a ser mostrar todas. A preferência é compartilhada com a janela de letras; o modo **Original + tradução** continua necessário. Versos sem correspondência de tradução permanecem somente no original. A legenda sobreposta CC não muda.

As duas opções são persistidas nas preferências existentes, sem alteração de schema. A medição do verso anterior considera a tradução para manter o verso em foco visível. A animação não altera o tempo da música ou os timestamps.

Verificação: 183 testes de biblioteca/binário e Clippy em todos os targets aprovados. Os novos testes cobrem valores padrão, persistência, limites das transições, conclusão e prioridade de redução de movimento. A aparência das animações e a interação com o player real não foram verificadas manualmente nesta revisão.

A rolagem manual pausa o acompanhamento também no modo Deslizar. “Voltar ao verso atual” retoma a rolagem automática. Reduzir animações aplica a nova posição imediatamente.

A revisão da rolagem inclui testes de posição intermediária/final, interrupção por novo alvo, seek para trás e redução de movimento. Não houve conferência visual da rolagem com o player real.

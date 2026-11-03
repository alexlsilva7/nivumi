# Sincronização por música

Os atalhos de compensação afetam todas as apresentações sincronizadas da letra: painel expandido, faixa compacta, leitor separado e legenda flutuante.

- **Ctrl+Alt++**: adiantar a letra em 100 ms.
- **Ctrl+Alt+-**: atrasar a letra em 100 ms.
- **Ctrl+Alt+0**: zerar somente a compensação da música atual.

As combinações personalizadas no menu da legenda continuam válidas. O ajuste funciona mesmo com a legenda flutuante fechada. Sem música identificada, o comando não cria uma compensação. O áudio, seek, duração, relógio e timestamps originais da letra não são alterados.

A compensação é salva no estado existente, em `settings.lyrics.song_offsets_ms`, com limite de -10000 a +10000 ms. Título e artista identificam a faixa, ignorando caixa e espaços externos. Duração, player, capa e filtros de busca não fazem parte da chave: mudanças nesses metadados não descartam o ajuste. Versões com o mesmo título e artista compartilham o ajuste.

Trocar de faixa carrega a compensação correspondente ou zero. Zerar remove apenas a entrada atual. Perfis antigos continuam carregando sem mudança de schema; uma compensação global antiga da legenda não é atribuída automaticamente a uma música desconhecida.

O host mantém a posição compensada para o painel e o leitor. A legenda recebe a posição de reprodução original e a compensação da faixa, aplicada uma única vez. Alterações feitas pelo menu da legenda também atualizam as outras apresentações. Na troca de música, as preferências pendentes da janela são coletadas antes de substituir os metadados, evitando associar o ajuste à próxima faixa.

Verificação: 210 testes passaram (163 da biblioteca, 47 do binário); formatação e Clippy de todos os targets sem erros. Os novos cenários cobrem persistência JSON, isolamento entre músicas e artistas, reset individual, ausência de título, limites/overflow, mudança de duração, seek, tempo anterior ao primeiro verso e consistência entre painel, conteúdo do leitor e legenda sem compensação dupla. Atalhos físicos com o player real não foram exercitados nesta revisão.

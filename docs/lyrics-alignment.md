# Alinhamento entre letra e tradução — 30/10/2026

O merge sequencial associava traduções ao LRC pela posição na lista. Uma diferença na divisão dos versos ou uma linha ausente deslocava as traduções seguintes. O alinhamento agora usa o original do provedor da tradução como ponte: original do Letras → versos com timestamp → tradução.

## Comportamento

- Busca uma sequência de correspondências pelo texto original, ignorando caixa, espaços e pontuação. Mantém a ordem dos versos e das repetições.
- Aceita um verso dividido em até três linhas em qualquer uma das fontes. Se um verso do Letras corresponde a várias linhas sincronizadas, a tradução desse bloco acompanha cada parte. Se várias linhas do Letras correspondem a uma linha sincronizada, suas traduções aparecem juntas.
- Pula versos sem correspondência sem deslocar os seguintes. Não altera timestamps nem o texto exibido do original.
- Prefere coletar o original da mesma página que contém a tradução, tanto no worker nativo quanto na extensão.
- Refaz o alinhamento quando os provedores chegam em qualquer ordem e remove associações antigas antes de tentar novamente.
- Mantém o modo **Original + tradução** ao carregar letras sincronizadas, mesmo sem matches. Todos os versos originais continuam visíveis; somente os que receberam correspondência exibem tradução.
- Exige que o original e a tradução da mesma fonte tenham a mesma quantidade de versos úteis. Se essa relação for ambígua, mantém a tradução completa disponível no modo de tradução, mas não a associa aos timestamps.

Isso corrige o pareamento; um atraso constante da letra em relação ao áudio continua sendo tratado pelo ajuste de sincronização existente.

## Limites

O algoritmo não deduz equivalência semântica entre textos diferentes, não expande indicações como “refrão 2x” e não inventa a divisão de uma tradução. Alterações nas palavras ou na edição da música podem deixar versos sem tradução sincronizada. Repetições idênticas sem contexto suficiente continuam ambíguas. Para limitar o trabalho na UI, importações com mais de 600 versos úteis por fonte ou verso normalizado acima de 2048 bytes não recebem alinhamento automático.

A extensão precisa ser recarregada e as abas do Letras atualizadas para usar a nova preferência de coleta. O aplicativo em execução não é substituído por esta mudança de código.

## Verificação

Os testes cobrem versos ausentes em ambas as fontes, versos divididos e unidos, repetições com contexto, pontuação, linhas instrumentais, diferenças de texto mesmo com contagens iguais, estrutura bilíngue ambígua, limpeza de associações anteriores, original em LRC e ordem de chegada dos provedores.

Na cópia isolada do HEAD com somente esta correção, `cargo fmt --check` passou e `cargo test --locked --quiet` passou com 86 testes (75 da biblioteca e 11 do binário). `cargo clippy --locked --all-targets -- -D warnings` e `node --check chrome-extension/content/letras.js` também passaram.

A tentativa inicial de verificar o workspace completo encontrou erros de compilação em outras alterações em andamento no `main.rs`. Elas foram preservadas e não fazem parte desta correção. A validação automatizada não comprova a sincronização durante reprodução real nem a disponibilidade dos provedores.

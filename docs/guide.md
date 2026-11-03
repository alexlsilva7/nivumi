# Guia do Nivumi

Aplicativo nativo para Windows que reúne música, letras, foco, hidratação, tarefas, calendário e arquivos em um painel compacto no topo da tela. Identidade própria, sem mascote, com monograma branco com recorte verde.

## Compilar e Executar

### Executar em Desenvolvimento (Build & Run)

Para compilar rapidamente e já iniciar o aplicativo:

```powershell
# Compilação incremental rápida em Debug e execução imediata
./scripts/run.ps1

# Parâmetros úteis de desenvolvimento
./scripts/run.ps1 -Inspect -Expanded -Page 0  # Fixado no topo, expandido na página do Player
./scripts/run.ps1 -KillExisting               # Fecha instância anterior antes de recompilar
./scripts/run.ps1 -Release                    # Compilação otimizada em Release
```

Também é possível invocar diretamente via CMD: `.\scripts\run.cmd`.

### Verificações Rápidas de Qualidade

```powershell
./scripts/check.ps1          # Roda fmt --check, clippy e testes
./scripts/check.ps1 -Fix     # Aplica formatação automática
```

### Empacotar e Distribuir (Release)

Requisitos: Rust com target `x86_64-pc-windows-msvc`, ferramentas C++ MSVC e Windows SDK. O ambiente de desenvolvimento usa Rust 1.99 e a biblioteca `windows` 0.62.2.

```powershell
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

O executável é gerado em `target/release/nivumi.exe`. Também é possível executar `scripts/build.ps1` (ou `scripts/build.cmd`), que verifica o projeto e copia o release e seu hash para `dist/`. Para iniciar o app automaticamente após o empacotamento, use `./scripts/build.ps1 -Run`.

Para gerar o instalador por usuário, use um compilador Inno Setup já disponível:

```powershell
./scripts/build.ps1 -Installer -IsccPath 'C:\caminho\Inno Setup 6\ISCC.exe'
```

O script também procura `ISCC.exe` no PATH, nas variáveis `NIVUMI_ISCC_PATH` e, por compatibilidade, `NUSA_ISCC_PATH`, nos caminhos padrão do Inno Setup 6/7 e no compilador oficial extraído neste workspace (`analysis/tools/inno-6.7.0/{app}/ISCC.exe`). Não instala ferramentas no sistema.

## Dados

### Versão portátil

Execute `./scripts/build.ps1` para gerar `dist/Nivumi-Portable.zip`.
Extraia o ZIP em uma pasta com permissão de escrita e abra `nivumi.exe`, sem instalar.
O arquivo `portable.flag` ao lado do executável ativa o armazenamento em `data/state.json` nessa mesma pasta. Feche o aplicativo antes de copiar a pasta inteira para outro computador. O pacote não inclui dados pessoais existentes.
A variável `NIVUMI_DATA_DIR` tem prioridade; `NUSA_REBUILD_DATA_DIR` permanece como alias de compatibilidade. Ambas precedem o modo portátil. Arquivos da Prateleira são referências aos caminhos originais e não são incluídos no pacote. Se ativar a inicialização com o Windows, desative-a antes de mover a pasta, pois ela registra o caminho atual no computador.

Configurações, tarefas, hidratação e referências da Prateleira ficam em `%LOCALAPPDATA%\NusaRebuild\state.json`. Há versão de schema, backup e gravação atômica. Falhas na leitura não devem substituir silenciosamente dados incompatíveis ou corrompidos.

Adicionar arquivos à Prateleira guarda referências às pastas originais. Remover um item retira apenas essa referência. Exportar com o botão esquerdo copia; arrastar com o direito permite escolher copiar ou mover no Explorador. Mover ou recortar/colar é uma operação real sobre o arquivo, solicitada pelo usuário. O projeto não importa, modifica ou remove dados do Nusa original.

Para testes isolados, defina `NIVUMI_DATA_DIR` somente no processo de teste antes de iniciar o aplicativo. Essa opção redireciona o diretório de estado.

## Integrações

- Win32, Direct2D, DirectWrite, DirectComposition e Direct3D 11 para janela e desenho.
- APIs Windows para sessões de mídia, volume, energia e conectividade. Cada player precisa disponibilizar metadados e comandos ao sistema.
- LRCLIB para letras sincronizadas, com tentativa alternativa no Lyricsify; Letras.mus.br para original e tradução em português. As consultas transmitem os metadados da faixa necessários à pesquisa. Ausência de letras, bloqueios dos sites ou conexão indisponível são tratados sem inventar resultados.
- OLE para arrastar arquivos/texto, com preservação dos arquivos de origem.
- Diálogos nativos com campos, checkboxes, combos e navegação por teclado.

Em **Configurações → Busca de letras**, adicione termos extras para ignorar nos títulos, uma expressão por linha. A lista começa com `(Live In Tampa 2022)` e pode ser editada ou esvaziada. A comparação ignora maiúsculas e minúsculas; os filtros de vídeo já existentes continuam ativos. Ao salvar, a busca da música atual é refeita e os títulos exibidos pelo player são preservados.

Nas telas **Música** e **Letras**, clique na capa, no título ou no artista para abrir o aplicativo que fornece a faixa atual. Janelas minimizadas são restauradas. Para mídia do navegador, o Nivumi abre a janela do navegador, dando preferência àquela cujo título contém o nome da faixa; não troca a aba ativa.

No notch expandido, nos modos **Letra completa** e **Tradução completa**, use a roda do mouse sobre o texto para rolar a letra. Fora da área do texto, a roda continua navegando entre páginas. Shift+roda ou Alt+roda também permitem rolar as letras, inclusive nos modos sincronizados.

Informações dependentes do hardware, como brilho e bateria Bluetooth, não devem ser simuladas quando o driver não as fornece. A presença de uma configuração não é comprovação de suporte ao hardware.

## Distribuição local

A instalação é por usuário, em `%LOCALAPPDATA%\Programs\Nivumi` para novas instalações (atualizações preservam a pasta anterior), sem exigir administrador. A inicialização com o Windows é uma opção do aplicativo, inicialmente desativada.

A desinstalação interativa pergunta antes de remover dados próprios. Em modo silencioso, preserva os dados. Não há publicação em loja, assinatura comercial, atualização automática ou sincronização de calendário externo nesta entrega.

O instalador Nivumi é gerado com Inno Setup 6.7.0. Consulte `docs/nivumi-branding.md` para os testes atuais de distribuição; `docs/dialogs.md` preserva os resultados históricos anteriores à nova marca.


### Prateleira

Com o notch em foco, **Ctrl+V** cola uma imagem da área de transferência (por exemplo, uma captura feita com Win+Shift+S) na Prateleira e abre essa página. PNG é preservado; bitmaps do Windows são salvos como BMP. As capturas ficam em `shelf-captures/`, junto ao arquivo de estado do Nivumi, e podem ser abertas, copiadas ou arrastadas como outros arquivos. Remover o item da Prateleira mantém o arquivo salvo.

Clique para selecionar, Ctrl para alternar itens e Shift para selecionar um intervalo. Ctrl+A seleciona todos; Delete remove as referências selecionadas. Cada cartão tem um botão de remoção individual. Duplo clique abre arquivos/pastas/links ou copia textos. A roda percorre os itens sem sair da página; Alt+roda mantém a navegação entre páginas.

Arraste com o botão esquerdo para copiar. Com o direito, o destino compatível oferece copiar ou mover. Os cartões selecionados se agrupam em 140 ms e o cursor mostra uma pilha com contador. Escape cancela o arraste. Imagens compatíveis com o WIC recebem miniaturas proporcionais carregadas em segundo plano.

O menu de contexto oferece abrir, copiar, recortar, remover e selecionar todos. Ctrl+C/Ctrl+X prepara arquivos para colar no destino com Ctrl+V. Seleções somente de textos/links são copiadas ou arrastadas como texto combinado; seleções que misturam arquivos e textos precisam ser exportadas separadamente.

### Tradução offline opcional

Em **Configurações → Letras**, use **Baixar e ativar** e salve as alterações. O pacote inglês → português brasileiro (~200 MB) é opcional; traduções humanas têm prioridade. Resultados locais recebem o indicador **Tradução offline**. [Detalhes, medidas e limitações](offline-translation.md).

## Identidade Nivumi e compatibilidade

A sincronização da letra é ajustável por música: **Ctrl+Alt++** adianta 0,1 s, **Ctrl+Alt+-** atrasa 0,1 s e **Ctrl+Alt+0** zera o ajuste da faixa atual. Vale para o painel de letras, modo compacto, leitor separado e legenda flutuante. [Detalhes e verificação](lyrics-sync.md).

A interface mantém as cores clássicas: preto, branco, verde, azul e vermelho conforme a função; o estado recolhido vazio exibe um monograma estático. As cores das capas continuam presentes no player e no leitor. O app, as janelas e a bandeja usam ícones próprios embutidos. [Mudanças e validação](nivumi-branding.md).

O perfil `%LOCALAPPDATA%\NusaRebuild` continua sendo usado por compatibilidade, sem mover dados, capturas, caches ou modelos. AppId do instalador, mutex e identificação de protocolo legados são preservados. Não há importação dos dados do Nusa original.

Gerador dos ícones (somente desenvolvimento):

```powershell
cargo run --manifest-path tools/brand-assets/Cargo.toml --target-dir target/brand-assets --locked
```

O instalador cria o atalho na Área de Trabalho por padrão (opção desmarcável). Ao concluir, a opção de fixação abre o atalho do menu Iniciar no Explorador: clique com o botão direito e escolha **Fixar na barra de tarefas**, usando **Mostrar mais opções** se necessário. A fixação exige ação do usuário; o instalador não altera as fixações existentes.

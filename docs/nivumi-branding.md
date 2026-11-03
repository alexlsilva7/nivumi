# Identidade Nivumi — implementação e verificação

Data: 02/11/2026. Branch: `codex/nivumi-identidade-visual`. Versões mantidas: app 0.2.0, extensão 1.2.1.

## Atualização: cores clássicas

Por solicitação posterior, a interface voltou às cores anteriores: notch preto, textos brancos, tarefas verdes, hidratação e tradução ativa azuis, calendário azul e pomodoro vermelho com pausas verdes. Configurações e extensão recuperaram suas cores anteriores. O nome Nivumi, a ausência de mascote e a geometria do monograma foram preservados; a logo e todos os ícones agora usam branco com recorte verde sobre preto. Os fundos das capas continuam dinâmicos.

Verificação desta revisão: formatação, Clippy sem avisos, 210 testes, build release e instalador. As 27 capturas foram regeneradas nas três escalas do aplicativo; a montagem foi conferida. Permanecem os limites de interação e DPI físico descritos abaixo. O registro de implementação seguinte descreve a primeira aplicação, cuja paleta lavanda/menta foi substituída por esta revisão.

## Histórico da primeira implementação

Nome Nivumi, monograma SVG próprio e paleta grafite/lavanda/menta/creme aplicados à interface e à extensão. O notch vazio usa um símbolo estático; desenho dos olhos, piscadas, detecção de repouso exclusiva do mascote e configuração de mascote foram removidos. Formato, navegação, dimensões e recursos das páginas foram preservados. A tradução ativa usa menta e o pomodoro distingue foco lavanda e pausas menta.

Os fundos continuam seguindo a capa. A intensidade é limitada proporcionalmente, preservando a tonalidade e reservando contraste para o texto secundário e para a onda de luz. Controles claros usam conteúdo grafite. Preferências de movimento reduzido continuam válidas.

Os recursos do executável incluem dois grupos de ícones e informações de produto Nivumi. O carregador mantém os handles vivos na thread de interface, seleciona dimensões por DPI, configura ícones pequenos/grandes das janelas e atualiza os ícones quando o DPI muda. A bandeja usa o símbolo transparente, também ao receber `TaskbarCreated`. Identidade de agrupamento: `Nivumi.Desktop`, a mesma usada nos atalhos do instalador.

Gerador de assets: [instruções](../assets/brand/README.md). `resvg` pertence somente à ferramenta de desenvolvimento. `embed-resource` pertence somente ao build. O comando de geração foi repetido e os assets permaneceram idênticos em bytes.

## Compatibilidade e distribuição

- Executável `nivumi.exe`; portátil `Nivumi-Portable.zip`; instalador `NivumiSetup.exe`.
- AppId do instalador e mutex de instância única legados mantidos. Instalação nova usa `Programs\Nivumi`; atualização respeita a pasta anterior.
- Perfil continua em `%LOCALAPPDATA%\NusaRebuild`. Estado, backups, capturas, referências da Prateleira, caches e modelos não são movidos nem apagados pelo rebranding.
- Precedência: `NIVUMI_DATA_DIR`, alias `NUSA_REBUILD_DATA_DIR`, `portable.flag` com pasta `data`, perfil legado. Schema permanece 1; o campo antigo `mascot` é ignorado na leitura.
- A entrada Run legada continua única. Instalador atualiza seu destino somente quando já existe; execução normal pode atualizar uma entrada habilitada, sem criar uma ausente. Perfis de teste com override não atualizam a entrada real.
- Executável e atalhos antigos são removidos depois de o novo payload estar instalado. Desinstalação silenciosa preserva dados; a interativa mantém a pergunta existente.
- Ponte continua em `127.0.0.1:48123`, com as mesmas rotas, formatos, identificador `nusa-notch` e ações internas legadas da extensão. Textos visíveis usam Nivumi.

## Verificações executadas

- `cargo fmt --check`, Clippy com `--all-targets -- -D warnings`, 207 testes (161 da biblioteca e 46 do binário), build release e compilação do instalador Inno Setup 6.7.0.
- Leitura/gravação de perfil legado com campo `mascot`, preservação de preferências e backup; prioridades dos diretórios; ida e volta dos 27 campos de configurações após a remoção da opção.
- Contraste dos tokens em superfícies e botões; limite da iluminação por capa. Essa verificação numérica não substitui revisão com leitores de tela.
- ICOs: nove quadros por arquivo, dimensões e conteúdo PNG conferidos. Handles pequenos/grandes/bandeja carregados no Windows em DPI 96/144/192; dimensões dos bitmaps dos ícones pequenos conferidas.
- Metadados PE: `FileDescription` e `ProductName` Nivumi, `OriginalFilename` nivumi.exe e versão 0.2.0.
- ZIP portátil contém apenas executável, marcador e instruções; o hash do executável no ZIP corresponde ao release.
- JavaScript da extensão passou em `node --check`; manifest JSON e caminhos dos ícones conferidos. Testes da ponte cobrem rotas e transferência de letras UTF-8 fragmentadas.
- Instalador exercitado com AppId, atalhos, pasta e entrada Run exclusivos de verificação: instalação nova, desinstalação silenciosa e atualização a partir da receita anterior com startup habilitado e desabilitado. Conferidos hash do payload instalado, remoção do executável antigo, pasta anterior, destino da entrada Run, destino/AppUserModelID dos atalhos de Iniciar e preservação de arquivo de estado sentinela. Nenhuma instalação real do usuário foi substituída por esses testes.

Reproduzir distribuição e instalação isolada:

```powershell
./scripts/build.ps1 -Installer
./scripts/verify-installer.ps1
```

O segundo script usa a receita de teste em `scripts/fixtures/legacy-installer.iss` e o binário atual renomeado como payload legado. Verifica as regras de atualização e distribuição; não é uma execução do aplicativo antigo com dados reais. Arquivos e resultados ficam em `dist/installer-verification/`, ignorados pelo Git.

## Capturas e limites da conferência visual

Foram geradas 27 capturas da superfície real Direct2D/Direct3D, em escalas do aplicativo de 100%, 150% e 200%, com movimento reduzido: compacto vazio, player vazio, música, letras/tradução, pomodoro, hidratação, Prateleira, tarefas e calendário. A composição usa fundo creme onde a janela é transparente. Músicas, letras e tarefa são dados sintéticos exclusivos da verificação.

```powershell
./scripts/capture-brand.ps1
```

Capturas e montagem: `dist/verification/*.png` e `dist/verification/overview.png`, ignoradas pelo Git. O modo `--verify-brand` e a captura de GPU existem somente em builds de desenvolvimento, exigindo perfil isolado; não são incluídos no release.

A ferramenta de Computer Use falhou antes de inicializar, com `failed to write kernel assets` / erro Windows 3, inclusive após reiniciar o kernel. Portanto **não foram concluídos** os seguintes testes de interação/integração visual: clique e teclado completos, Narrator, arraste real entre apps, extensão carregada no Chrome, configurações e leitor separado operados manualmente, mudança física de DPI/monitor, aparência ao vivo no Explorador/barra de tarefas/Alt+Tab e reinício real do Explorer. As capturas não comprovam esses cenários. O carregamento de recursos e o código de recuperação da bandeja foram verificados separadamente; o Explorer do usuário não foi reiniciado.

Fixações antigas na barra de tarefas podem continuar apontando para o nome anterior; se houver um atalho antigo fixado, deve ser substituído por uma fixação do Nivumi. O instalador atualiza seus atalhos de Iniciar/Área de Trabalho; não manipula o banco privado de fixações do Windows.

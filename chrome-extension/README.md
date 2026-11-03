# Nivumi Lyrics Bridge · Extensão do Google Chrome

Extensão oficial para Google Chrome que conecta os sites **[Letras.mus.br](https://www.letras.mus.br)** e **[Lyricsify.com](https://www.lyricsify.com)** diretamente ao aplicativo desktop **Nivumi** (Notch no Windows).

---

## 🚀 Funcionalidades

1. **Extração de Letras Sincronizadas com Tempos (Lyricsify.com)**:
   - Ao acessar qualquer música no `lyricsify.com` (ou nas buscas com botões `[ ⏱️ Nivumi Sync ]`), a extensão extrai o formato LRC com timestamps precisos (`[mm:ss.xx]`).
   - Widget flutuante `⏱️ Enviar LRC ao Nivumi` para envio instantâneo com 1 clique.
2. **Combinação de Letra em Inglês com Tradução PT-BR**:
   - O Nivumi alinha automaticamente a letra em inglês sincronizada por tempo (do Lyricsify ou LRCLIB) com a tradução em português estrofe por estrofe do Letras.mus.br.
   - Modo `[Sync+Trad]`: a linha ativa em inglês toca no tempo da música com a respectiva tradução em português exibida logo abaixo em destaque ciano!
3. **Extração de Letras e Traduções do Letras.mus.br**:
   - Ao acessar qualquer página do `letras.mus.br` (ex: SIAMÉS - "Mr. FEAR"), extrai a letra original e a tradução em português simultaneamente.
4. **Removedor de Adblock & Modais Promocionais**:
   - Remove o modal de aviso de bloqueador de anúncios (`.fc-ab-root` / Funding Choices) e banners promocionais (`.contextualPlanModal`).
   - Esconde players flutuantes intrusivos (`.player-media`, iframes de terceiros) e destrava o scroll para navegação livre.
5. **Pesquisa Rápida via Menu de Contexto (YouTube e Web)**:
   - Selecione qualquer nome de música ou trecho, clique com o botão direito e escolha `"Buscar letra no Letras.mus.br"` ou `"Buscar letra com tempos no Lyricsify"`.
6. **Popup com Detecção de Mídia do Windows**:
   - Conversa com o Nivumi na porta `48123` e descobre qual música está tocando no seu computador (Spotify, YouTube, player local).
   - Botões de 1 clique: `🔎 Letras.mus.br` e `⏱️ Lyricsify (LRC)`.
7. **Abas no Desktop Notch (Nivumi)**:
   - `[Sync]`: Letras sincronizadas clássicas em inglês.
   - `[Sync+Trad]`: Letras sincronizadas em inglês + tradução em português em tempo real!
   - `[Letras]`: Letra original completa com rolagem (scroll do mouse ou botões ▲/▼).
   - `[Tradução]`: Tradução em português completa com rolagem.

---

## 📦 Como Instalar no Google Chrome

1. Abra o Google Chrome e acesse a página de extensões digitando na barra de endereços:
   ```text
   chrome://extensions
   ```
2. No canto superior direito, ative a chave **"Modo do desenvolvedor"** (*Developer mode*).
3. Clique no botão **"Carregar sem compactação"** (*Load unpacked*) no canto superior esquerdo.
4. Navegue até a pasta do projeto Nivumi e selecione o diretório:
   ```text
   chrome-extension
   ```
5. Pronto! O ícone do **Nivumi Lyrics Bridge** aparecerá na barra de ferramentas do seu Chrome. Fixe-o na barra para fácil acesso.

---

## 🛠️ Comunicação Local (API REST)

O aplicativo desktop Nivumi roda um servidor HTTP ultraleve em background na porta local `127.0.0.1:48123`:
- `GET http://127.0.0.1:48123/api/status`: Handshake de status.
- `GET http://127.0.0.1:48123/api/current`: Retorna a música atual tocando no Windows.
- `POST http://127.0.0.1:48123/api/lyrics`: Envia a letra e a tradução para exibição no Notch.

## Correção de letras (1.2.1)

Recarregue a extensão em `chrome://extensions` e atualize as abas dos provedores após substituir os arquivos. O app busca primeiro no LRCLIB e tenta Lyricsify como alternativa. Se o Lyricsify exigir verificação no navegador, a busca automática pode ficar bloqueada; o envio pela extensão depende de a página acessível conter LRC. A tradução vem do Letras. Resultados vazios não apagam uma letra sincronizada importada, e o original/tradução do Letras são preservados.

A validação desta revisão cobre os parsers com HTML de teste, a ponte TCP com letras grandes e a consulta real LRCLIB/Letras. O envio pela extensão em um navegador interativo não foi validado de ponta a ponta nesta revisão.

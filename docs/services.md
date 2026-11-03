# Serviços Windows e OLE

`ServiceHandle::spawn()` cria um worker MTA, com `tx` para `ServiceCommand` e `rx` para `ServiceEvent`. A thread da interface deve drenar os eventos sem bloquear. O `Drop` do handle envia `Stop`.

## Mídia

GSMTC obtém a sessão atual escolhida pelo Windows, metadados, duração, posição, estado, capacidades e capa (limite de 4 MiB). Callbacks de mudança de sessão, propriedades, reprodução e timeline sinalizam atualização; heartbeat de 2 s recupera notificações perdidas e atualiza posição. A UI pode extrapolar posição a partir de `rate` enquanto `playing` for verdadeiro.

A capa é relida ao mudar de faixa, sessão ou propriedades de mídia. Mesmo com metadados estáveis, o worker a verifica novamente a cada 5 s, na próxima atualização de mídia, para substituir ícones provisórios ou capas atrasadas do navegador. Falhas de leitura são tentadas novamente após 2 s: a capa atual é preservada apenas para a mesma faixa, sem transportar a imagem da música anterior. A cor de destaque só é recalculada quando os bytes da imagem mudam.

`PlayPause`, `Next`, `Previous` e `Seek(f64)` retornam erros pela fila de eventos. A UI deve respeitar `can_toggle`, `can_next`, `can_previous`, `can_seek`. `Volume(f32)` usa Core Audio na saída multimídia padrão, consultada novamente para acompanhar troca de dispositivo.

## Letras

`FetchLyrics {title, artist, duration}` usa um worker MTA separado. Limpa metadados de vídeo (VEVO, Topic, Official Video), consulta LRCLIB `/api/search` por HTTP WinRT identificado e valida título, artista, LRC não vazio e duração (diferença máxima de 10 s quando conhecida). Se não houver resultado utilizável ou ocorrer erro, tenta a página pública correspondente no Lyricsify. O Lyricsify não é tratado como uma API JSON; o parser extrai LRC dos contêineres HTML. Caso o HTTP falhe, tenta o navegador local em perfil separado. Uma página de verificação do Cloudflare é reportada como bloqueio, sem tentativa de resolvê-la.

Cada HTTP tem prazo de 12 s. O navegador usa `--timeout=12000` para capturar o DOM mesmo com recursos pendentes e um limite externo de 20 s para encerrar o processo se necessário. O fallback pode somar esses prazos; não existe um prazo global de 12 s para toda a cadeia. O cache `lyrics/v2-*.json` guarda apenas LRC validado e sua fonte, por chave da faixa; entradas antigas sem validação não são reutilizadas.

O evento `Lyrics {key,text,error}` contém a chave original e `text` inclui o texto e a fonte. A UI descarta chaves diferentes de `MediaSnapshot::key()`. Respostas ausentes ou vazias não apagam uma importação sincronizada já recebida. A chegada do resultado automático após o Letras seleciona Sync ou Sync+Trad conforme a tradução disponível.

`FetchLetrasLyrics` roda em outro worker. Seus eventos `LetrasLyrics {key,lyrics}` usam título/artista originais e são descartados após troca de faixa. A tradução continua vindo do Letras: somente os contêineres de tradução são lidos, sem recolher versos do original nem parágrafos do restante da página. A mesclagem preserva original/tradução do Letras quando chega LRC do Lyricsify, em qualquer ordem. O alinhamento da tradução permanece por sequência de versos não vazios; versões com cortes, versos adicionais ou diferente divisão podem exigir ajuste.


## Sistema

O snapshot é consultado a cada 2 s e só enviado quando muda: bateria/energia (`GetSystemPowerStatus`), volume (Core Audio), Caps Lock, idioma do teclado da janela em foco, conectividade WinRT, brilho WMI. O serviço não registra teclas digitadas.

Brilho: conexão persistente a `ROOT\\WMI`, consulta `WmiMonitorBrightness WHERE Active=TRUE`, limite de espera de 500 ms na enumeração. Hardware sem esse provedor retorna `None`; não há fallback DDC/CI para telas externas nesta implementação.

Bluetooth: enumeração dos nomes conectados a cada 10 s, usando seletor do Windows. Não abre conexão com controles ou dispositivos arbitrários. Percentual de bateria Bluetooth permanece `None`: falta uma fonte universal comprovada e nenhum número é simulado. Os nomes podem incluir dispositivos que não são fones; a UI não deve apresentar todo dispositivo como headset.

`set_startup(bool)` grava/remove apenas `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\\NusaRebuild`, com o caminho do próprio executável. `enum_monitors()` retorna nomes, limites e tela principal.

## Shelf / OLE

Inicializar OLE no STA da interface antes de `register_drop_target(hwnd, sender)`. Guardar `DropRegistration` durante a vida da janela: o objeto revoga o registro ao ser descartado. O canal entrega `DropPayload::Files(Vec<PathBuf>)` ou `Text(String)`.

`file_data` cria um `IDataObject` nativo por `SHCreateShellItemArrayFromIDLists` e `BHID_DataObject`, incluindo arquivos de pastas diferentes. `drag_files_with_button` limita o botão esquerdo a COPY e permite COPY/MOVE no direito, com `IDropSource` acompanhando o botão iniciador. `IDragSourceHelper::InitializeFromBitmap` fornece a pilha no cursor. O destino decide se aceita a operação ou apresenta o menu do botão direito. O DropTarget da Prateleira continua aceitando somente COPY para adicionar referências.

`clipboard_files` usa Preferred DropEffect e o clipboard OLE para copiar/recortar. `drag_text_with_button` combina textos/links em `CF_UNICODETEXT`. Cancelamento não remove referências. Após MOVE confirmado, referências cujos caminhos originais desapareceram são removidas. A UI nunca apaga o arquivo de origem manualmente; a operação pertence ao Shell/destino.

## Limites e validação

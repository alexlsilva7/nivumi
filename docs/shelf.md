# Prateleira: comportamento e UX

- Clique seleciona; Ctrl alterna; Shift seleciona intervalo; Ctrl+A seleciona todos. Arrastar um item já selecionado preserva o grupo.
- Duplo clique abre; textos são copiados. Remover no cartão, Delete e Remover no cabeçalho retiram referências. Um clique no fundo limpa a seleção; arrastar o fundo continua movendo o notch.
- Exportação começa após 6 DIP de movimento e agrupamento de 140 ms. Movimento reduzido pula a animação. O cursor recebe uma pilha com contador (limitado a 99 na imagem do cursor).
- Esquerdo copia. Direito oferece COPY/MOVE ao destino; no Explorador isso permite escolher a operação. Escape cancela. O Shell gerencia a transferência, sem exclusão manual pelo aplicativo.
- Roda percorre a lista; Ctrl+C/Ctrl+X e menu de contexto dão alternativas ao arraste. A miniatura mantém proporção e não lê arquivos no loop de desenho. Falhas e formatos sem codec mantêm ícone genérico.
- Seleções mistas de arquivos e textos não descartam itens silenciosamente: é preciso separar as operações. Textos/links selecionados são combinados com quebras de linha.

## Próximas melhorias de UX

1. Desfazer a remoção de referências, com Ctrl+Z e ação no aviso.
2. Prévia ampliada de imagens ao manter o cursor e indicação de arquivos ausentes.
3. Busca e filtros quando a lista crescer; fixar itens frequentes e indicar a posição na lista.
4. Sincronizar referências com o destino após movimentos externos ou transferências assíncronas.

## Referências técnicas

O [IDragSourceHelper](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-idragsourcehelper-initializefrombitmap) recebe um bitmap com alfa não pré-multiplicado. O objeto de dados nativo é criado pelo Shell para manter os formatos e a negociação da transferência.

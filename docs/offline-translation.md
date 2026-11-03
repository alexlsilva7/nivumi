# Tradução offline opcional

## Experiência de download nas configurações

A seção Letras separa a opção de uso, o pacote e os filtros de busca. O pacote mostra idioma, status, barra de progresso e botão de ação independentes. Há estados explícitos de preparação, transferência, verificação/instalação, cancelamento em andamento, download cancelado, falha e pacote instalado. Erros aparecem na própria seção; cancelamento não abre um alerta de erro. Arquivos completos já baixados continuam reutilizáveis na próxima tentativa.

Durante o download, Salvar fica indisponível para evitar encerrar a janela antes da instalação. A interface informa que fechar cancela o download. A opção de uso só fica disponível com o pacote instalado e é marcada ao concluir; a ativação continua dependendo de Salvar. Ao reabrir, um pacote existente aparece como pronto e o botão de download fica indisponível. A prévia de configurações agora sempre usa uma pasta temporária própria, sem acessar os dados do usuário.

Verificação desta revisão: Clippy em todos os targets passou antes dos últimos ajustes de ordem de tabulação e dos três testes de feedback. A prévia visual confirmou os controles nativos, barra de progresso e reconhecimento do pacote instalado; nela foi identificado e corrigido o desenho que usava o rótulo Cancelar para o botão de pacote. A compilação e os testes finais foram impedidos pela falta de memória virtual do Windows (erro 1455, falhas de mmap e LLVM out of memory), inclusive usando um único processo. Não foi gerado novo executável desta revisão. Os testes adicionados cobrem limites do progresso, cancelamento pendente e a distinção entre download completo e instalação concluída.

Em Configurações → Letras, use **Baixar e ativar** para instalar o pacote inglês → português brasileiro (~199,2 MB de transferência). A opção será marcada após a conclusão; clique em Salvar alterações para ativar. Cancelar as configurações preserva o pacote instalado, sem ativar a preferência. Desativar e salvar oculta traduções locais, cancela trabalhos pendentes e preserva o cache.

O Nivumi prioriza traduções humanas. Ao terminar a busca no Letras sem tradução, traduz o original disponível, desde que o detector local reconheça inglês com confiança. Sem original disponível, não há tradução. O recurso não baixa letras nem transcreve áudio offline. Letras sincronizadas já armazenadas podem ser utilizadas sem rede. Os modos com tradução mostram “Tradução offline” na notch e no leitor; a legenda recebe os mesmos versos.

## Implementação

Modelo: Xenova/opus-mt-en-ROMANCE, revisão 9d2ba69ac80c8e8453c3d9a1e2323a0e7b8ca3cd, encoder e decoder mesclado quantizados. Tokenização SentencePiece de origem/destino com vocabulário compartilhado e token >>pt_br<<. Inferência autoregressiva com KV cache, por verso, com deduplicação de refrões e divisão de entradas longas. A geração aborta em vez de truncar silenciosamente ao atingir o limite do decoder.

ONNX Runtime CPU 1.31.1, ort 2.0.0-rc.15, carregamento por caminho absoluto. Um worker serial limita a CPU a duas threads e mantém sessões por até 60 segundos de inatividade. A DLL pode permanecer mapeada no processo após liberar as sessões. Não requer Python, GPU ou conta externa.

O manifesto assets/offline-translation.json fixa URLs HTTPS, tamanhos e hashes SHA-256. Os arquivos são verificados antes da instalação atômica; arquivos incompletos ficam em staging. Ao tentar novamente, arquivos já completos e válidos são reutilizados. O pacote fica em models/en-pt-br-v1 junto ao estado, respeitando modo portátil e NUSA_REBUILD_DATA_DIR. Cache de tradução independente em lyrics/offline-v1-*.json inclui identidade da música, texto original e versão do pacote. O cache v3 de letras permanece compatível.

Licenças: modelo Helsinki-NLP/OPUS-MT Apache-2.0; ONNX Runtime MIT (avisos do runtime são extraídos do arquivo oficial). Fontes: https://huggingface.co/Helsinki-NLP/opus-mt-en-ROMANCE e https://github.com/microsoft/onnxruntime/releases/tag/v1.31.1 . O download consulta os servidores Hugging Face e GitHub; a inferência utiliza apenas arquivos locais. As buscas humanas existentes continuam consultando seus provedores.

## Verificação em 01/11/2026

170 testes passaram, além de formatação e Clippy em todos os targets. Há cobertura de configuração antiga, persistência da opção, identificação de idioma, identidade do cache, hashes/tamanhos inválidos, indicador por modo e preservação/substituição da procedência na mesclagem de provedores.

O probe examples/offline_translation_probe.rs instalou o pacote real num perfil isolado e traduziu texto sintético com linha vazia e refrão repetido. A inferência em release, sem solicitar downloads ou buscas, levou 1,13 segundo incluindo carregamento, com pico observado de 391,8 MiB. Executável medido: 3.325.952 bytes (~3,3 MB). Essas medidas dependem do hardware e do texto; não representam tempo garantido para uma música inteira.

A prévia isolada examples/offline_settings_preview.rs confirmou a seção Letras, seus rótulos acessíveis, ativação, download e texto de ajuda. A prévia foi fechada sem salvar. Não foi alterada a conectividade do computador para testar uma desconexão física. Não foi exercitado o download completo por cliques, a matriz de DPI, nem o fluxo de reprodução real de ponta a ponta. Um teste encontrou bloqueio do executável da prévia ainda aberto e passou após fechá-lo. O release principal também estava em uso; a compilação final utiliza target/offline-release sem encerrar o aplicativo do usuário.

Comandos: cargo fmt --check; cargo test --locked; cargo clippy --locked --all-targets -- -D warnings; cargo build --release --locked --target-dir target/offline-release.

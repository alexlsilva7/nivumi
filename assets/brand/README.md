# Assets Nivumi

Monograma geométrico original baseado na direção visual aprovada: n branco com recorte diagonal verde (`#30D158`). Sem personagem ou rosto.

- `mark.svg`: símbolo transparente de 24 unidades.
- `mark-small.svg`: desenho com hastes mais largas para 16–24 px.
- `mark-mono.svg`: versão monocromática com `currentColor`.
- `icon.svg`: aplicação sobre placa preta de cantos arredondados.
- `app.ico`: ícone do executável, janelas e atalhos, com placa preta.
- `tray.ico`: símbolo transparente para a área de notificação.
- `icon-{size}.png`: versões raster para uso em ferramentas.

Os ICOs contêm 16, 20, 24, 32, 40, 48, 64, 128 e 256 px, com quadros PNG RGBA de 32 bits.

Regenerar a partir da raiz:

```powershell
cargo run --manifest-path tools/brand-assets/Cargo.toml --target-dir target/brand-assets --locked
```

O gerador usa resvg somente no desenvolvimento. Não é dependência normal do app. O build Windows utiliza os ICOs versionados e embute os recursos em cada executável, inclusive testes e exemplos nativos. Regenerar os arquivos preserva seus bytes quando os SVGs e o gerador não mudam.

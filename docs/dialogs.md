# Diálogos e instalação

O Nivumi usa diálogos Win32 para configurações e formulários, com navegação por teclado.

A receita `packaging/installer.iss` gera uma instalação por usuário, sem privilégios administrativos, com atalhos no menu Iniciar e, por padrão, na Área de Trabalho. A fixação na barra de tarefas é feita pelo usuário ao concluir.

Atualizações preservam a pasta instalada, os dados e a preferência de inicialização. A desinstalação silenciosa mantém os dados; a interativa pergunta antes de removê-los. Arquivos referenciados na Prateleira não são apagados.

`scripts/verify-installer.ps1` testa instalação, atualização e desinstalação com identificadores e pastas isolados. A receita em `scripts/fixtures/legacy-installer.iss` existe exclusivamente para testar compatibilidade de atualização.

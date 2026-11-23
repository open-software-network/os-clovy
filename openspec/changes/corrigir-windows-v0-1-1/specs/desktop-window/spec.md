## Purpose

Define o que acontece com a janela principal do Clovy ao fechar e como ela volta, no Windows e no macOS, enquanto o app segue rodando em segundo plano.

## ADDED Requirements

### Requirement: Fechar esconde a janela principal
Fechar a janela principal (botão de fechar ou menu Fechar) SHALL escondê-la sem destruí-la, em todos os sistemas; o app continua rodando em segundo plano com o ícone na bandeja (Windows) ou na barra de menus (macOS).

#### Scenario: Fechar no Windows
- **WHEN** o usuário clica no X da janela principal no Windows
- **THEN** a janela some, o app continua na bandeja e nenhuma gravação ou ditado em curso é interrompido
- Test: unit — tratamento do pedido de fechar registrado em todos os sistemas; manual — Windows de prova

### Requirement: Reabrir a janela principal
A janela principal SHALL voltar visível, restaurada e com foco quando o usuário clica no ícone da bandeja (Windows), escolhe "Abrir Clovy" no menu do ícone, abre o app de novo (segunda abertura) ou usa qualquer ação que leve à janela (ajustes, nova sessão, notificação). Se a janela não existir mais, o app MUST recriá-la.

#### Scenario: Clique no ícone da bandeja
- **WHEN** a janela está escondida e o usuário clica com o botão esquerdo no ícone do Clovy na bandeja do Windows
- **THEN** a janela principal aparece na frente, com foco
- Test: manual — Windows de prova

#### Scenario: Abrir o app de novo
- **WHEN** a janela está escondida e o usuário abre o Clovy pelo menu Iniciar
- **THEN** nenhuma segunda instância fica rodando e a janela da primeira aparece com foco
- Test: unit — callback de instância única chama o mesmo caminho de mostrar janela; manual — Windows de prova

#### Scenario: Janela destruída
- **WHEN** a janela principal não existe mais e uma ação pede para mostrá-la
- **THEN** o app recria a janela a partir da configuração e a mostra
- Test: unit — caminho único de mostrar janela com recriação, usado por bandeja, menu, instância única, notificações e HUDs

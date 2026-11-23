# Spec Delta

## Purpose

Define quando o Clovy aparece no Dock do macOS e permite rodar só pela barra de menus enquanto a janela principal está fechada.

## ADDED Requirements

### Requirement: Preferência de ocultar do Dock
No macOS, o Clovy SHALL oferecer em Configurações → Geral a preferência "Ocultar do Dock", desligada por padrão. A preferência MUST ser gravada localmente e MUST valer após reiniciar o app. Em outras plataformas a preferência MUST não aparecer.

#### Scenario: Valor padrão
- **WHEN** o usuário abre Configurações → Geral pela primeira vez no macOS
- **THEN** "Ocultar do Dock" aparece desligada
- Test: unit — carga do arquivo de preferências ausente; component — linha em Geral

#### Scenario: Preferência persiste
- **WHEN** o usuário liga "Ocultar do Dock" e reinicia o Clovy
- **THEN** a preferência continua ligada
- Test: unit — ida e volta do arquivo de preferências

#### Scenario: Arquivo corrompido
- **WHEN** o arquivo de preferências tem conteúdo inválido
- **THEN** o app abre com a preferência desligada, sem erro
- Test: unit — carga de arquivo malformado

### Requirement: Ícone fora do Dock com a janela fechada
Com a preferência ligada, ao esconder a janela principal o Clovy SHALL sair do Dock e do Cmd-Tab e MUST continuar rodando com o ícone da barra de menus. Ao mostrar a janela de novo, por qualquer caminho, o ícone MUST voltar ao Dock e a janela MUST receber foco.

#### Scenario: Fechar a janela
- **WHEN** a preferência está ligada e o usuário fecha a janela principal
- **THEN** o ícone sai do Dock e o ícone da barra de menus continua disponível
- Test: unit — decisão de política (preferência ligada, janela escondida); e2e — app de desenvolvimento conferido com Computer Use

#### Scenario: Reabrir pela barra de menus
- **WHEN** o ícone está fora do Dock e o usuário escolhe "Abrir Clovy" na barra de menus
- **THEN** a janela aparece com foco e o ícone volta ao Dock
- Test: unit — decisão de política (janela visível); e2e — app de desenvolvimento conferido com Computer Use

#### Scenario: Preferência desligada
- **WHEN** a preferência está desligada e o usuário fecha a janela
- **THEN** o ícone continua no Dock, como antes
- Test: unit — decisão de política (preferência desligada)

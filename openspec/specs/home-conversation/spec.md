# home-conversation Specification

## Purpose
Define o que o Início (a conversa persistente da tela inicial do Clovy) faz com uma mensagem do usuário: qual motor responde, onde a conversa fica guardada, como ela continua entre mensagens e reinícios, e como falhas aparecem.

## Requirements

### Requirement: Início responde no motor CLI padrão
Toda mensagem enviada no Início SHALL ser respondida por um turno do motor CLI padrão do chat (Claude Code; Codex só quando o Claude Code não está no PATH do shell de login), pelo mesmo caminho de turno das sessões CLI, no modo de segurança `sandboxed`. O Início MUST NOT chamar a Clovy API para responder.

#### Scenario: Mensagem simples com Claude Code instalado
- **WHEN** o usuário envia "eae" no Início e o Claude Code está instalado e logado
- **THEN** a resposta do Claude Code aparece em streaming na conversa do Início
- **AND** nenhuma requisição é feita a `/v1/chat/completions` da Clovy API

#### Scenario: Clovy API sem chave de provedor
- **WHEN** a Clovy API local não tem chave da OpenRouter e o usuário envia uma mensagem no Início
- **THEN** a resposta vem do Claude Code normalmente, sem `home_chat_failed`

#### Scenario: Só o Codex instalado
- **WHEN** o Claude Code não está no PATH e o Codex está
- **THEN** a mensagem do Início roda num turno do Codex

Test: integration — CLI falso no lugar do `claude` grava argv e devolve stream; servidor HTTP falso da Clovy API conta zero chamadas; component — Início renderiza a resposta do turno.

### Requirement: Trabalho acontece no próprio Início
O Início SHALL deixar o motor CLI responder e usar suas ferramentas ali mesmo, com aprovações e cartões iguais aos de uma sessão CLI. O Início MUST NOT classificar a mensagem, forçar pesquisa nem abrir automaticamente uma sessão separada.

#### Scenario: Pedido de informação atual
- **WHEN** o usuário pergunta "qual a previsão do tempo amanhã em SP?" no Início
- **THEN** o pedido vai como turno do motor CLI na conversa do Início, sem criar outra sessão

#### Scenario: Ferramenta que pede aprovação
- **WHEN** o turno do Início pede uma ação que exige aprovação
- **THEN** o cartão de aprovação aparece na conversa do Início como numa sessão CLI

Test: component — envio no Início chama o turno da sessão do Início e não cria sessão nova.

### Requirement: Conversa do Início é persistida e contínua
As mensagens do Início e suas respostas SHALL ficar gravadas como itens da sessão persistente do Início no banco do perfil, e cada nova mensagem MUST continuar a mesma conversa do motor (retomada da conversa do CLI ou histórico prefixado quando não houver o que retomar). A sessão do Início MUST continuar fora da lista de sessões da barra lateral.

#### Scenario: Continuidade entre mensagens
- **WHEN** o usuário diz "meu nome é Gui" e depois pergunta "qual meu nome?"
- **THEN** o segundo turno recebe a conversa anterior e responde com o nome

#### Scenario: Reiniciar o app
- **WHEN** o usuário fecha e reabre o Clovy
- **THEN** a conversa do Início reaparece igual, lida do banco

#### Scenario: Barra lateral
- **WHEN** o Início tem mensagens
- **THEN** a sessão do Início não aparece em "Sessões" na barra lateral

Test: integration — dois turnos com CLI falso, o segundo recebe `--resume` ou o histórico; component — Início recarregado mostra os itens; sidebar sem a sessão do Início.

### Requirement: Histórico antigo do Início é preservado
Na primeira abertura após a mudança, as mensagens diretas do Início que estavam guardadas só no armazenamento local do webview SHALL ser importadas uma única vez, em ordem, como itens da sessão do Início, e os cartões de tarefas já entregues MUST continuar visíveis e abrindo a sessão de destino. O armazenamento local antigo MUST NOT ser apagado.

#### Scenario: Importação única
- **WHEN** o Início abre com mensagens antigas no armazenamento local e a sessão do Início ainda não tem itens importados
- **THEN** as mensagens aparecem na conversa na mesma ordem e o próximo turno as recebe como histórico

#### Scenario: Reabrir não duplica
- **WHEN** o Início é aberto de novo depois da importação
- **THEN** nenhuma mensagem é importada outra vez

#### Scenario: Cartão de tarefa antigo
- **WHEN** existe um cartão de tarefa entregue antes da mudança
- **THEN** o cartão aparece e abre a sessão que ele aponta

Test: integration — fixture de armazenamento local com turnos e cartões; abrir duas vezes; contagem de itens estável.

### Requirement: Falhas do motor no Início são acionáveis
Quando nenhum motor CLI está instalado, o CLI não está logado ou o turno falha, o Início SHALL mostrar o mesmo erro traduzido de uma sessão CLI e MUST manter a mensagem do usuário recuperável no compositor.

#### Scenario: Nenhum CLI instalado
- **WHEN** nem Claude Code nem Codex estão no PATH e o usuário envia uma mensagem no Início
- **THEN** o Início mostra o erro de motor ausente das sessões CLI e a mensagem continua no compositor

#### Scenario: Turno falha
- **WHEN** o CLI termina com erro no meio do turno do Início
- **THEN** o erro aparece na conversa como numa sessão CLI e o usuário pode tentar de novo

Test: component — erro do turno mapeado pelo catálogo de erros das sessões CLI; compositor preserva o texto.

### Requirement: Comandos de triagem do Início não existem
O desktop MUST NOT expor os comandos `clovy_home_chat` e `june_home_chat`, e nenhum código de produção SHALL invocá-los.

#### Scenario: Registro de comandos
- **WHEN** a guarda de registro de comandos Tauri roda
- **THEN** nenhum `invoke` de `clovy_home_chat` ou `june_home_chat` existe no frontend e nenhum dos dois está no `generate_handler!`

Test: static — `src/test/tauri-command-registry.test.ts` e busca no código.

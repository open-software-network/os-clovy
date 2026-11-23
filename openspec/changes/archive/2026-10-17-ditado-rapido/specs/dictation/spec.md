# Spec Delta

## Purpose

Define como o ditado do Clovy começa, transcreve, opcionalmente limpa e entrega o texto, com metas de latência que o tornam tão ágil quanto colar o transcrito do Whisper direto.

## ADDED Requirements

### Requirement: Limpeza do ditado é opcional e desligada por padrão
O ditado SHALL ter um controle de limpeza do texto em Configurações, desligado por padrão. Com a limpeza desligada, o texto colado MUST ser o transcrito do Whisper, sem nenhuma chamada de modelo de linguagem.

#### Scenario: Limpeza desligada
- **WHEN** a limpeza está desligada e o usuário dita uma frase
- **THEN** o transcrito do Whisper é colado e nenhuma chamada de LLM é feita

#### Scenario: Instalação existente com limpeza em CLI ou endpoint
- **WHEN** o app abre pela primeira vez após a mudança e a limpeza estava configurada para um CLI ou endpoint local
- **THEN** a limpeza fica desligada e essa migração não se repete

Test: integration — tomada com transcritor falso e limpeza desligada não chama o limpador; fixture de `provider-settings.json` com `dictationCleanup` em `cli` migra para desligado uma vez.

### Requirement: Limpeza ligada usa só OpenRouter em chamada simples
Com a limpeza ligada, o texto SHALL ser limpo por um modelo da OpenRouter escolhido pelo usuário, numa chamada única de chat completions por pedaço de texto. A limpeza MUST NOT iniciar CLI de agente, sessão de runtime nem endpoint local.

#### Scenario: Escolha de motor
- **WHEN** o usuário abre a escolha de modelo da limpeza
- **THEN** só modelos da OpenRouter aparecem; CLIs e endpoints locais não aparecem

#### Scenario: Gravação de configuração inválida
- **WHEN** algo tenta gravar um CLI ou endpoint como motor da limpeza
- **THEN** a gravação é recusada com erro

#### Scenario: Falha da limpeza
- **WHEN** a chamada de limpeza falha ou estoura o prazo
- **THEN** o transcrito sem limpeza é colado e o usuário vê um aviso discreto

Test: unit — normalização do registro recusa `cli`/endpoint para a limpeza; integration — limpador falso que falha cola o transcrito bruto.

### Requirement: Transcrição direta com chave própria
Quando o usuário tem a própria chave do Groq salva, a transcrição SHALL ir direto do desktop ao Groq; quando tem a própria chave da OpenRouter, a limpeza ligada SHALL ir direto à OpenRouter. Sem chave própria, cada chamada MUST seguir pela Clovy API como antes. A chave MUST ir só ao provedor dela.

#### Scenario: Com chave Groq própria
- **WHEN** a chave do Groq está salva e o usuário dita
- **THEN** a requisição vai a `api.groq.com` e nenhuma requisição de transcrição vai à Clovy API

#### Scenario: Sem chave própria
- **WHEN** não há chave do Groq salva
- **THEN** a transcrição vai a `/v1/dictate` da Clovy API

Test: integration — servidores HTTP falsos no lugar de Groq e Clovy API contam requisições nos dois casos; cabeçalhos levam só a chave do provedor chamado.

### Requirement: Gravação começa no aperto do atalho
Com o app aberto e pronto, a captura SHALL começar em até 150 ms depois do aperto do atalho, inclusive no primeiro aperto após abrir o app (até 300 ms). O som de início MUST NOT atrasar a captura e MUST NOT aparecer no transcrito.

#### Scenario: Aperto com app já aberto
- **WHEN** o usuário aperta fn com o app aberto há mais de 5 segundos
- **THEN** `listening_started` é registrado até 150 ms depois de `shortcut_key_down`

#### Scenario: Primeiro aperto após abrir
- **WHEN** é o primeiro aperto depois de abrir o app
- **THEN** `listening_started` vem até 300 ms depois de `shortcut_key_down`

Test: manual — `dictation-events.log` do app de prova com 10 tomadas; integration no helper — captura iniciada antes do fim do som de início.

### Requirement: Entrega rápida sem limpeza
Com a limpeza desligada e chave própria do Groq, o texto de uma tomada de até 10 s SHALL ser colado em até 1,5 s depois de soltar o atalho, sem espera fixa antes de colar quando o app de destino já está ativo. A detecção de "nenhuma fala" MUST NOT rodar antes do envio ao Whisper.

#### Scenario: Tomada curta
- **WHEN** o usuário dita cerca de 8 s e solta o atalho
- **THEN** `paste_completed` vem até 1,5 s depois de `shortcut_key_up`

#### Scenario: Silêncio
- **WHEN** a tomada não tem fala e o Whisper devolve texto vazio
- **THEN** nada é colado e o usuário vê "nenhuma fala detectada"

Test: manual — `dictation-events.log` com 10 tomadas; unit no helper — colagem sem espera quando o destino já está ativo.

### Requirement: Posição do indicador do ditado
O indicador do ditado (pílula do HUD) SHALL aparecer centralizado na horizontal, na base da área útil da tela (acima do Dock) por padrão. Configurações SHALL oferecer a escolha "Embaixo" ou "Em cima"; "Em cima" coloca o indicador logo abaixo da barra de menus. A posição MUST valer para a tela onde está o cursor e MUST NOT cair no meio da tela.

#### Scenario: Padrão
- **WHEN** o usuário aperta o atalho sem ter mudado a posição
- **THEN** o indicador aparece centralizado, encostado na base da área útil com a margem padrão

#### Scenario: Em cima
- **WHEN** o usuário escolhe "Em cima" e dita
- **THEN** o indicador aparece centralizado logo abaixo da barra de menus

Test: unit — cálculo da origem para Embaixo/Em cima em telas com e sem Dock e com escala 1x/2x; manual — frame da janela `Clovy Dictation` lido por `CGWindowListCopyWindowInfo` no Mac do dono nas duas opções.

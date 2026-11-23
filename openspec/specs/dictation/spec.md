# dictation Specification

## Purpose
Define como o ditado do Clovy começa, transcreve, opcionalmente limpa e entrega o texto, com metas de latência que o tornam tão ágil quanto colar o transcrito do Whisper direto.

## Requirements

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
A transcrição do ditado SHALL ir ao motor de transcrição escolhido em Configurações: com Groq, sempre direto do desktop à Groq com a chave do usuário; com Local, ao modelo instalado na máquina, sem rede. A limpeza ligada SHALL ir sempre direto à OpenRouter. Não existe rota de reserva entre motores. Com o motor Groq e sem a chave Groq, o ditado MUST mostrar que a chave falta e onde salvá-la, sem requisição de rede; com o motor Local, a chave Groq MUST NOT ser exigida. Sem a chave OpenRouter, a limpeza é pulada e o transcrito bruto é colado com aviso discreto. A chave MUST ir só ao provedor dela.

#### Scenario: Com chave Groq própria
- **WHEN** o motor é Groq, a chave do Groq está salva e o usuário dita
- **THEN** a requisição vai a `api.groq.com` e nenhuma outra requisição de transcrição é feita

#### Scenario: Sem chave própria
- **WHEN** o motor é Groq, não há chave do Groq salva e o usuário aperta o atalho do ditado
- **THEN** o HUD mostra que falta a chave Groq e onde salvá-la, e nenhuma requisição sai da máquina

#### Scenario: Motor Local
- **WHEN** o motor é Local e o usuário dita
- **THEN** a tomada é transcrita pelo modelo local, o texto é colado e nenhuma requisição de transcrição sai da máquina, haja ou não chave Groq salva

#### Scenario: Limpeza ligada sem chave OpenRouter
- **WHEN** a limpeza está ligada, a transcrição teve sucesso e não há chave OpenRouter
- **THEN** o transcrito bruto é colado e o usuário vê o aviso de chave OpenRouter ausente

Test: integration — servidores HTTP falsos no lugar de Groq e OpenRouter e um servidor sentinela em porta efêmera, injetada como a URL legada, que MUST receber zero requisições; cabeçalhos levam só a chave do provedor chamado; com motor Local, transcritor local falso e zero requisições à Groq falsa.

Nota de verificação (orquestrador, 2026-10-28): os testes automatizados usam porta efêmera para evitar conflitos locais. A prova nativa em `127.0.0.1:8080` é separada, executada somente com a porta livre e limitada aos fluxos autorizados, conforme a tarefa 7.2 e seu relatório. Isso não declara sucesso dos fluxos reais de provedores adiados para validação manual pós-merge pelo dono.

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

### Requirement: Reinserção explícita do último ditado no macOS
No macOS, Control+V SHALL reinserir o último texto válido de ditado disponível nesta execução e no perfil ativo, no campo atualmente focado. A reinserção MUST funcionar independentemente do clipboard, sem alterar seu conteúdo, reativar o destino antigo ou iniciar transcrição, limpeza ou ação de agente.

#### Scenario: Colagem automática não entrou no campo
- **WHEN** uma transcrição válida já terminou, o clipboard anterior foi restaurado e o usuário foca outro campo e aperta Control+V
- **THEN** o último ditado é inserido uma vez no campo atual e o clipboard permanece inalterado

#### Scenario: Conteúdo Unicode
- **WHEN** o último ditado contém acentos, emoji e quebras de linha e o usuário aperta Control+V
- **THEN** a inserção preserva esse texto, sem substituições por combinações de teclas ou truncamento

#### Scenario: Colagem normal do sistema
- **WHEN** o usuário aperta Command+V no macOS
- **THEN** o macOS continua colando o conteúdo normal do clipboard, sem disparar a reinserção

### Requirement: Memória do último ditado respeita validade e isolamento
O Clovy SHALL manter somente em memória um último ditado elegível, associado ao perfil que o produziu. Um novo resultado válido SHALL substituir o anterior. Silêncio, cancelamento, texto rejeitado e resultado tardio sem autoridade MUST NOT substituir essa memória. Reinserir MUST NOT criar outro item de histórico.

#### Scenario: Falha de colagem não perde o resultado válido
- **WHEN** a transcrição de um ditado válido terminou mas sua colagem automática falhou
- **THEN** esse texto continua disponível para Control+V

#### Scenario: Tomada cancelada depois de um ditado válido
- **WHEN** existe um ditado elegível e outra tomada é cancelada, vazia ou rejeitada
- **THEN** Control+V continua usando o ditado elegível anterior

#### Scenario: Outro perfil ou execução sem ditado
- **WHEN** o perfil ativo não é o dono do texto em memória ou o app acabou de iniciar
- **THEN** Control+V não insere texto nem recupera ditados do disco ou de outro perfil

### Requirement: Atalho de reinserção preserva os controles nativos
A reinserção SHALL exigir a Acessibilidade existente e respeitar a captura de atalhos. O app MUST NOT pedir Input Monitoring para esse recurso nem registrar Control+V no Windows/Linux. Conflitos MUST preservar atalhos de gravação existentes e reportar a indisponibilidade do atalho de reinserção, sem registrá-lo silenciosamente como funcional.

#### Scenario: Permissão ausente
- **WHEN** Control+V é acionado sem Acessibilidade
- **THEN** nenhum texto é inserido nem copiado e o app sinaliza a permissão ausente pelo caminho existente

#### Scenario: Captura ou conflito de atalho
- **WHEN** o usuário está capturando um atalho ou Control+V conflita com outro atalho de gravação
- **THEN** não ocorre reinserção indevida e os controles existentes continuam utilizáveis

### Requirement: Destino do ditado fixado no acionamento do atalho
No macOS, ao acionar o atalho de ditado, o Clovy SHALL capturar o destino da tomada: o app externo da frente e, quando a Acessibilidade permitir, sua janela e o campo focados. Esse destino SHALL valer para a fala, a finalização, a limpeza e a colagem automática da tomada. Ativação, abertura ou clique em outra janela durante a tomada MUST NOT alterar o destino nem o ícone do indicador. O stop MUST NOT fixar o destino de novo.

#### Scenario: Outra janela abre durante a fala
- **WHEN** o usuário aperta o atalho num campo do app A e, com o atalho pressionado, outra janela de outro app passa para a frente
- **THEN** o indicador continua mostrando o app A e, ao soltar, o texto é colado no campo original do app A

#### Scenario: Outra janela do mesmo app
- **WHEN** a tomada começou numa janela do app A e outra janela do próprio app A passa para a frente durante a fala
- **THEN** a colagem traz a janela original para a frente e cola nela, não na janela nova

#### Scenario: Nenhuma trava fora da tomada
- **WHEN** nenhuma tomada de ditado está ativa e o usuário troca de janela
- **THEN** o candidato a destino acompanha o foco normalmente, e a próxima tomada captura o destino em vigor no seu acionamento

#### Scenario: Tomada termina ou é cancelada
- **WHEN** a tomada é entregue, cancelada, rejeitada ou fica sem fala
- **THEN** o destino fixado é descartado e não é reutilizado por outra tomada

### Requirement: Destino da tomada indisponível não cola em outro lugar
Se o app, a janela ou o campo da tomada não existir mais ou não puder ser trazido para a frente até o prazo de ativação, o Clovy MUST NOT colar em outro app ou janela. O texto SHALL seguir a recuperação existente: permanecer no clipboard com o aviso atual e continuar disponível para o Control+V. Sem Acessibilidade para ler a janela, o destino SHALL se limitar ao app capturado no acionamento.

#### Scenario: Janela original fechou durante a fala
- **WHEN** a janela capturada no acionamento foi fechada antes da entrega
- **THEN** nada é colado, o aviso de destino indisponível é mostrado e o texto continua recuperável por Command+V e Control+V

#### Scenario: App não volta para a frente
- **WHEN** o app ou a janela da tomada não fica na frente até o prazo de ativação
- **THEN** o Command+V não é enviado e o texto segue a recuperação existente

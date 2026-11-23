# Spec Delta

## Purpose

Permite que o usuário instale um modelo de fala no próprio Mac e o escolha como motor de todas as transcrições do Clovy, sem que o áudio saia da máquina.

## ADDED Requirements

### Requirement: Disponibilidade do motor local por plataforma
O motor de transcrição Local SHALL ser oferecido só em macOS 14 ou superior com Apple Silicon. Em Windows e em Mac Intel, a linha de Transcrição das Configurações MUST mostrar só a Groq, e uma configuração salva com motor Local MUST ser lida como Groq, sem erro.

#### Scenario: Mac com Apple Silicon
- **WHEN** o usuário abre Configurações > Modelos num Mac com Apple Silicon e macOS 14+
- **THEN** a linha de Transcrição oferece os motores Groq e Local

#### Scenario: Windows
- **WHEN** o usuário abre Configurações > Modelos no Windows
- **THEN** a linha de Transcrição oferece só a Groq e nenhum controle de instalação de modelo aparece

#### Scenario: Configuração salva com Local em plataforma sem suporte
- **WHEN** `provider-settings.json` tem `transcriptionProvider: "local"` e o app abre numa plataforma sem suporte
- **THEN** o motor efetivo é Groq e a interface mostra Groq selecionado

Test: unit (Rust) — sanitização por plataforma; unit (frontend) — linha de Transcrição com flag de plataforma.

### Requirement: Instalação do modelo com progresso
Configurações SHALL ter um botão "Instalar modelo" que baixa o pacote do modelo local (revisão fixa) para o diretório de dados do app, mostrando progresso em bytes recebidos sobre o total, com cancelamento. Cada arquivo MUST ter o SHA-256 conferido antes de o modelo ser dado como pronto; um arquivo divergente MUST deixar o modelo em "falhou" com motivo, sem apagar os arquivos já íntegros.

#### Scenario: Instalação completa
- **WHEN** o usuário clica "Instalar modelo" e o download termina com todos os arquivos íntegros
- **THEN** o estado passa a "pronto", o tamanho ocupado aparece ao lado e o motor Local fica selecionável

#### Scenario: Progresso visível
- **WHEN** o download está em andamento
- **THEN** a interface mostra os bytes recebidos sobre o total e um botão "Cancelar"; o botão "Instalar modelo" não aparece

#### Scenario: Cancelamento
- **WHEN** o usuário clica "Cancelar" durante o download
- **THEN** o download para, o estado volta a "ausente" e nenhuma transcrição é afetada

#### Scenario: Arquivo corrompido
- **WHEN** um arquivo baixado tem SHA-256 diferente do esperado
- **THEN** o estado passa a "falhou" com o nome do arquivo no motivo, o botão "Tentar de novo" aparece, e a nova tentativa baixa só o que falta ou divergiu

#### Scenario: Sem rede
- **WHEN** o download falha por rede
- **THEN** o estado passa a "falhou" com mensagem de rede na língua da interface e "Tentar de novo" retoma do que já foi baixado

Test: integration (Rust) — servidor HTTP falso com arquivos de tamanho conhecido: sucesso, hash divergente, cancelamento e retomada; unit (frontend) — estados ausente/baixando/pronto/falhou da linha.

### Requirement: Remoção do modelo
Com o modelo pronto, Configurações SHALL oferecer "Remover modelo", que apaga os arquivos do modelo. Se o motor Local estava escolhido, a remoção MUST trocar o motor para Groq e dizer isso ao usuário.

#### Scenario: Remover com Local escolhido
- **WHEN** o motor é Local e o usuário remove o modelo
- **THEN** os arquivos são apagados, o motor passa a Groq e a interface avisa que as transcrições voltam a usar a Groq

#### Scenario: Remover com Groq escolhido
- **WHEN** o motor é Groq e o usuário remove o modelo
- **THEN** os arquivos são apagados e o motor continua Groq

Test: integration (Rust) — remoção limpa o diretório e grava a configuração.

### Requirement: Estado do modelo persistente
O estado do modelo (ausente, baixando, pronto, falhou) SHALL ser derivado dos arquivos em disco ao abrir o app: um diretório com todos os arquivos íntegros é "pronto"; um download interrompido por fechamento do app é "ausente" com retomada possível. O estado MUST ser consultável pela interface a qualquer momento.

#### Scenario: Reabrir com modelo pronto
- **WHEN** o app abre e o diretório do modelo está completo
- **THEN** Configurações mostra "pronto" sem baixar nada

#### Scenario: Reabrir após download interrompido
- **WHEN** o app fecha no meio do download e reabre
- **THEN** Configurações mostra "ausente" com "Instalar modelo", e a instalação reaproveita os arquivos íntegros já baixados

Test: integration (Rust) — diretório parcial e completo em pasta temporária.

### Requirement: Escolha do motor Local exige modelo pronto
O motor Local SHALL ser selecionável só com o modelo em "pronto". Uma tentativa de gravar `transcriptionProvider: "local"` sem modelo pronto MUST ser recusada com erro, e a interface MUST mostrar a opção desabilitada com a instrução de instalar.

#### Scenario: Seleção sem modelo
- **WHEN** o modelo está ausente e o usuário abre a escolha de motor
- **THEN** "Local" aparece desabilitado com o texto "Instale o modelo para usar"

#### Scenario: Gravação recusada
- **WHEN** algo tenta gravar o motor Local sem modelo pronto
- **THEN** a gravação falha com `local_model_not_installed` e a configuração anterior permanece

Test: unit (Rust) — `set_provider_model` com estado do modelo falso; unit (frontend) — opção desabilitada.

### Requirement: Transcrição local sem rede
Com o motor Local escolhido, toda transcrição (ditado, turnos e trechos de notas, pacotes de turnos, prévia ao vivo) SHALL ser feita pelo modelo local no dispositivo. Nenhuma requisição de rede MUST ser feita para transcrever, a chave Groq MUST NOT ser exigida, e o ritmo e a espera por limite de taxa da Groq MUST NOT se aplicar. Nunca há troca automática para a Groq.

#### Scenario: Ditado sem chave Groq
- **WHEN** o motor é Local, não há chave Groq salva e o usuário dita
- **THEN** o texto transcrito localmente é colado e nenhuma requisição de transcrição sai da máquina

#### Scenario: Nota longa
- **WHEN** o motor é Local e uma gravação de 40 minutos termina
- **THEN** todos os trechos são transcritos localmente, sem pausa por limite de taxa, e a nota é gerada

#### Scenario: Turnos locais sem mistura entre turnos
- **WHEN** o motor é Local e a transcrição final processa turnos de uma nota
- **THEN** cada turno é enviado numa requisição local individual e recebe somente o texto dessa requisição
- **AND** o caminho Local não agrega turnos em pacotes, evitando que tempos imprecisos e a agregação de tokens por pausa de 400 ms atribuam texto a outro turno
- **AND** o caminho Groq mantém seus pacotes com intervalo fixo de 500 ms entre turnos e divisão pelos tempos dos segmentos; essa regra não é alterada pelo motor Local

#### Scenario: Prévia ao vivo
- **WHEN** o motor é Local, a prévia ao vivo está ligada e um trecho fica pronto
- **THEN** o trecho é transcrito localmente e aparece na prévia, sem orçamento de segundos por hora

#### Scenario: Idioma do ditado
- **WHEN** o idioma do ditado está configurado como português
- **THEN** o modelo local recebe a dica de idioma e devolve o texto em português

Test: integration (Rust) — transcritor local falso nos caminhos de ditado, nota, turnos individuais e prévia, com servidor sentinela de rede recebendo zero requisições; regressão com três turnos e tempos imprecisos confere atribuição individual e preserva o intervalo de 500 ms dos pacotes Groq; unit — pacer não é consultado com motor Local.

### Requirement: Falha do motor local é erro claro
Quando o helper local não inicia, o modelo não carrega ou a transcrição falha, a nota ou o ditado SHALL falhar com `local_transcriber_failed` e uma mensagem na língua da interface dizendo que o modelo local falhou e sugerindo reinstalar ou trocar para a Groq. O áudio da nota fica salvo e "Tentar de novo" continua disponível. A falha MUST NOT redirecionar para a Groq.

#### Scenario: Helper não inicia
- **WHEN** o motor é Local e o helper não responde ao carregar o modelo
- **THEN** a nota mostra a mensagem do motor local, o áudio fica salvo e nenhuma requisição à Groq é feita

#### Scenario: Ditado com falha local
- **WHEN** o motor é Local e a transcrição de uma tomada falha
- **THEN** o HUD mostra que o modelo local falhou e nada é colado

Test: integration (Rust) — transcritor local falso que falha; unit (frontend) — `note-failure-banner` traduz a frase.

### Requirement: Modelo carregado uma vez por sessão
O helper local SHALL carregar o modelo na primeira transcrição ou ao escolher o motor Local e mantê-lo carregado enquanto o app estiver aberto, para que tomadas seguintes não paguem a carga de novo. Trocar o motor para Groq ou remover o modelo MUST liberar o helper e sua memória.

#### Scenario: Segunda tomada
- **WHEN** o usuário dita duas vezes seguidas com o motor Local
- **THEN** a segunda tomada não recarrega o modelo e o texto é colado em menos tempo que a primeira

#### Scenario: Troca para Groq
- **WHEN** o usuário troca o motor de Local para Groq
- **THEN** o helper local é encerrado

Test: integration (Rust) — cliente do helper com processo falso contando cargas; manual — `dictation-events.log` com duas tomadas.

# llm-providers Specification

## Purpose
Permite que o usuário use seus próprios provedores de IA no Clovy (CLIs de agentes locais e endpoints OpenAI-compatíveis) para geração pontual e como motor do chat, sem depender do Clovy API.

## Requirements

### Requirement: Catálogo de provedores CLI detectados
O sistema SHALL reconhecer os CLIs `claude`, `codex`, `pi`, `agy`, `cursor-agent` e `copilot`, resolvendo-os pelo PATH do shell de login do usuário (não apenas pelo PATH herdado pelo app gráfico) e exibindo, para cada um, se está instalado, o caminho resolvido e a versão quando o CLI a informar. O ambiente do perfil do usuário (por exemplo `HOME` real e variáveis como `PI_CODING_AGENT_DIR`) MUST ser preservado nas execuções. No Windows, que não tem shell de login, o sistema MUST resolver os CLIs pelo `Path` do ambiente do app, lido sem diferenciar maiúsculas de minúsculas, mais os diretórios onde os instaladores oficiais desses CLIs gravam; MUST aceitar só arquivos com extensão listada em `PATHEXT`; e MUST entregar ao CLI resolvido como `.cmd` ou `.bat` o prompt exatamente como foi composto.

#### Scenario: CLI instalado fora do PATH do app gráfico
- **WHEN** o `claude` existe apenas em um diretório adicionado pelo shell de login (por exemplo `~/.local/bin`) e o usuário abre a lista de provedores
- **THEN** o Clovy mostra `claude` como instalado com o caminho resolvido e permite selecioná-lo
- Test: unit — resolução de PATH via shell de login com executável falso em diretório temporário

#### Scenario: CLI ausente
- **WHEN** nenhum executável `cursor-agent` é encontrado
- **THEN** o provedor aparece como não instalado, não pode ser selecionado e o motivo é exibido
- Test: unit — detecção com PATH sem o executável

#### Scenario: CLI instalado pelo npm no Windows
- **WHEN** no Windows o `codex` foi instalado pelo npm, que grava `codex.cmd` e um script `codex` sem extensão em `%APPDATA%\npm`, e o ambiente do app traz esse diretório na variável `Path`
- **THEN** o Clovy mostra `codex` como instalado com o caminho de `codex.cmd` e a versão informada pelo CLI
- Test: unit — resolução com chave `Path`, `PATHEXT` e diretório temporário contendo `codex.cmd` e `codex` sem extensão

#### Scenario: CLI fora do Path no Windows
- **WHEN** no Windows o `claude.exe` existe só em `%USERPROFILE%\.local\bin` e esse diretório não está no `Path` do app
- **THEN** o Clovy mostra `claude` como instalado com esse caminho
- Test: unit — diretórios de reserva do Windows derivados de `USERPROFILE` e `APPDATA`

#### Scenario: Prompt íntegro num CLI `.cmd`
- **WHEN** no Windows um CLI resolvido como `.cmd` recebe um prompt com várias linhas, aspas, `%`, `^` e `&`, por argumento ou pela entrada padrão
- **THEN** o CLI recebe o texto byte a byte igual ao composto pelo Clovy, ou a execução falha com erro explícito, nunca com o texto alterado
- Test: integration (Windows) — `.cmd` falso que devolve o argumento e a entrada padrão recebidos

### Requirement: Endpoints OpenAI-compatíveis nomeados
O sistema SHALL permitir cadastrar vários endpoints OpenAI-compatíveis, cada um com nome, URL base, modelo e chave opcional. A chave MUST ficar no Keychain e nunca ser devolvida à interface nem registrada em log. O endpoint local já existente nas configurações do Clovy MUST ser migrado como um endpoint nomeado, sem perda de configuração.

#### Scenario: Migração do endpoint local existente
- **WHEN** o Clovy inicia com `provider-settings.json` contendo `localGeneration` preenchido
- **THEN** a lista de provedores contém um endpoint com a mesma URL, modelo e chave, e as rotas que usavam o endpoint local continuam usando-o
- Test: unit — carga de settings legado e verificação do registro migrado

#### Scenario: Chave não volta à interface
- **WHEN** a interface lista os endpoints cadastrados
- **THEN** cada endpoint informa apenas se possui chave, sem o valor
- Test: unit — DTO serializado sem o campo de segredo

### Requirement: Teste de conexão e capacidade de saída estruturada
O sistema SHALL oferecer, para cada provedor, um teste de conexão que executa um prompt mínimo e mostra sucesso com latência ou o erro recebido, e SHALL registrar o nível de saída estruturada suportado (`none`, `prompt`, `json_object`, `json_schema`, `strict`). Recursos que exigem JSON estruturado MUST recusar provedores abaixo de `json_schema` com mensagem explicativa.

#### Scenario: Teste de conexão bem-sucedido
- **WHEN** o usuário aciona "Testar" em um provedor configurado e acessível
- **THEN** a interface mostra sucesso com a latência medida e o nível de saída estruturada detectado
- Test: integration — CLI falso e servidor HTTP falso respondendo ao prompt de teste

#### Scenario: Provedor sem saída estruturada suficiente
- **WHEN** o usuário escolhe para os recursos de atividade um endpoint cujo nível detectado é `prompt`
- **THEN** a escolha é recusada e a interface explica que é necessário suporte a JSON schema
- Test: unit — validação de seleção por nível de capacidade

### Requirement: Seleção de provedor por uso
O sistema SHALL permitir escolher o provedor separadamente para: chat/agente, geração de notas, limpeza de ditado, transcrição de áudio e recursos de atividade (relatórios, resumo do dia, resumo de sessões de agentes). O chat MUST aceitar só motores CLI. As notas MUST aceitar CLI, endpoint cadastrado ou OpenRouter com a chave do usuário. A limpeza de ditado usa a OpenRouter. A transcrição MUST aceitar Groq (com a chave do usuário) ou Local (modelo instalado na máquina, só onde suportado); o padrão é Groq. Nenhum uso MUST passar pelo Clovy API. Para recursos de atividade, o padrão MUST ser "nenhum", e sem provedor escolhido esses recursos ficam desligados.

#### Scenario: Atividade sem provedor
- **WHEN** nenhum provedor de atividade foi escolhido e chega a hora de gerar o resumo do dia
- **THEN** nenhum texto de atividade sai da máquina e a interface mostra que o recurso precisa de um provedor configurado
- Test: integration — agendador com provedor "nenhum" não realiza chamada e registra estado desligado

#### Scenario: Nota gerada por CLI
- **WHEN** o provedor de notas é `claude` e uma gravação termina de ser transcrita
- **THEN** a nota estruturada é gerada pelo CLI e aparece no editor como as notas geradas pela OpenRouter
- Test: integration — CLI falso devolvendo nota e verificação da nota persistida

#### Scenario: Perfil salvo com uso Clovy
- **WHEN** um `provider-settings.json` salvo tem `notes` ou `dictationCleanup` com `kind: "clovy"` e o app abre
- **THEN** notas passam a OpenRouter e a limpeza segue na OpenRouter, sem chamada ao Clovy API, e o arquivo é regravado com o valor novo
- Test: integration — fixture de `provider-settings.json`, carga, gravação e recarga

#### Scenario: Transcrição Local escolhida
- **WHEN** o modelo local está pronto e o usuário escolhe Local na linha de Transcrição
- **THEN** `transcriptionProvider` é gravado como `local`, o modelo exibido é o do pacote instalado, e ditado, notas e prévia passam a usar o motor local
- Test: integration — gravação, recarga e leitura do motor efetivo

#### Scenario: Override de perfil sem modelo pronto
- **WHEN** um perfil tem override de transcrição para `local` e o modelo não está pronto
- **THEN** a transcrição desse perfil falha com `local_model_not_installed`, sem requisição à Groq, e a nota ou o ditado mostra a mensagem do motor local
- Test: unit — `effective_settings_for_profile` com estado do modelo falso; integration — servidor sentinela Groq com zero requisições

### Requirement: Execução isolada de CLIs em geração pontual
Chamadas de geração pontual a CLIs SHALL rodar sem persistir sessão no CLI, sem ferramentas e sem carregar arquivos de contexto/skills, com tempo limite, e MUST marcar o prompt com o marcador de autoria do Clovy para que a ingestão de sessões de agentes as ignore. CLIs que não garantem todas as ferramentas desligadas (hoje `codex`, `agy`, `cursor-agent`, `copilot`) MUST ser recusados para notas, limpeza de ditado e atividade, sem redirecionamento silencioso; continuam disponíveis como motor do chat. Chamadas em segundo plano dos recursos de atividade MUST ser serializadas (uma por vez).

#### Scenario: CLI que mantém ferramentas
- **WHEN** o usuário tenta escolher `codex` para notas
- **THEN** a escolha é recusada com a explicação de que as ferramentas desse CLI não podem ser desligadas, e nenhum processo do `codex` é iniciado
- Test: unit — seleção por uso e invocação recusadas com CLI falso instalado

#### Scenario: Marcador de autoria presente
- **WHEN** o Clovy executa qualquer chamada de geração pontual por CLI
- **THEN** o prompt enviado contém o marcador de autoria do Clovy
- Test: unit — montagem do comando/prompt por provedor

#### Scenario: Tempo limite
- **WHEN** um CLI não responde dentro do tempo limite configurado
- **THEN** o processo é encerrado e a chamada falha com erro de tempo esgotado, sem processo órfão
- Test: integration — CLI falso que dorme além do limite

### Requirement: CLI como motor do chat
O sistema SHALL permitir escolher um CLI como motor de uma sessão de chat. Nessa sessão, cada mensagem do usuário MUST ser entregue ao CLI com continuidade da conversa anterior, e texto, raciocínio e atividade de ferramentas emitidos pelo CLI MUST aparecer na interface de sessão do Clovy e ser persistidos como itens da sessão. Cancelar MUST encerrar o processo do CLI. Quando o CLI aceitar servidores MCP, as ferramentas do Clovy MUST ser oferecidas pelo servidor MCP do Clovy; quando não aceitar, a interface MUST indicar que a sessão não tem ferramentas do Clovy. Mensagens enviadas durante uma execução MUST virar mensagem seguinte em vez de serem perdidas.

#### Scenario: Conversa com continuidade
- **WHEN** o usuário envia duas mensagens seguidas numa sessão com motor `claude`
- **THEN** a segunda execução retoma a mesma conversa do CLI e ambas as respostas aparecem em ordem na sessão, persistidas após reiniciar o app
- Test: integration — CLI falso que exige o identificador de retomada na segunda chamada

#### Scenario: Cancelamento
- **WHEN** o usuário cancela uma execução em andamento com motor CLI
- **THEN** o processo do CLI termina e a execução fica marcada como cancelada
- Test: integration — CLI falso de longa duração e verificação de término do processo

### Requirement: Notas com chave própria vão direto ao provedor
Com o motor de Notas = OpenRouter, o desktop SHALL mandar a transcrição de nota ao motor de transcrição escolhido (Groq direto com a chave do usuário, ou Local no dispositivo) e a geração e a limpeza da nota à OpenRouter, sempre direto, com um modelo explícito. A requisição MUST levar só a chave do provedor chamado. Não existe rota de reserva: sem a chave, a chamada falha como descrito em "Falha clara sem chave nem servidor".

#### Scenario: Transcrição final com chave Groq
- **WHEN** o motor de transcrição é Groq, a chave Groq está salva e um turno da reunião é transcrito
- **THEN** a requisição vai a `/audio/transcriptions` da Groq com modelo, `response_format=json`, idioma, prompt do dicionário e nome de arquivo canônico
- Test: integration — Groq falsa confere o corpo multipart e os cabeçalhos

#### Scenario: Transcrição final com motor Local
- **WHEN** o motor de transcrição é Local e um turno da reunião é transcrito
- **THEN** o turno é transcrito pelo modelo local com a dica de idioma, e nenhuma requisição de transcrição sai da máquina
- Test: integration — transcritor local falso e Groq falsa com zero requisições

#### Scenario: Prévia ao vivo com chave Groq
- **WHEN** o motor de transcrição é Groq, a chave Groq está salva e a prévia ao vivo envia um trecho
- **THEN** o trecho vai direto à Groq
- Test: integration — pedido de prévia contra Groq falsa

#### Scenario: Prévia ao vivo com motor Local
- **WHEN** o motor de transcrição é Local, a prévia ao vivo está ligada e um trecho fica pronto
- **THEN** o trecho é transcrito localmente, sem orçamento de segundos por hora
- Test: integration — transcritor local falso

#### Scenario: Sem chave Groq
- **WHEN** o motor de transcrição é Groq, não há chave Groq salva e uma gravação começa e termina
- **THEN** a prévia ao vivo não inicia, a transcrição falha com `groq_api_key_missing` e nenhuma requisição de transcrição ou prévia é feita
- Test: unit — portão da prévia com e sem chave; integration — servidor sentinela em porta efêmera, injetada como a URL legada, com zero requisições

#### Scenario: Falha transitória da Groq
- **WHEN** a Groq responde 503 na primeira tentativa e 200 na segunda
- **THEN** a transcrição devolve o texto da segunda resposta depois de exatamente duas requisições; um 400 não é repetido
- Test: integration — Groq falsa com contador de chamadas

#### Scenario: Áudio sem fala
- **WHEN** a Groq devolve texto vazio, ou o modelo local devolve texto vazio
- **THEN** a transcrição falha com `no_speech`, que o pipeline trata como fonte sem fala
- Test: integration — Groq falsa; transcritor local falso

#### Scenario: Chave Groq rejeitada
- **WHEN** a Groq responde 401 ou 403
- **THEN** a transcrição falha com `groq_api_key_rejected` e a frase que manda atualizar a chave
- Test: integration — Groq falsa

#### Scenario: Áudio acima de 25 MB
- **WHEN** o motor é Groq e o arquivo do trecho tem mais de 26 214 400 bytes
- **THEN** a transcrição falha com `audio_too_large` sem nenhuma requisição; com motor Local esse limite não existe
- Test: integration — Groq falsa sem requisições; transcritor local falso com arquivo grande

#### Scenario: Geração com chave OpenRouter
- **WHEN** a chave OpenRouter está salva e o modelo de notas é explícito
- **THEN** a nota vem de uma chamada `/chat/completions` à OpenRouter com o prompt `note_generate.md` e os rótulos de fonte limpos
- Test: integration — OpenRouter falsa

#### Scenario: Limpeza da transcrição da nota com chave OpenRouter
- **WHEN** a chave OpenRouter está salva e o pipeline limpa a transcrição com o estilo `note_transcript_cleanup`
- **THEN** a chamada vai à OpenRouter com o prompt `note_transcript_cleanup.md` e `temperature: 0`
- Test: integration — OpenRouter falsa

#### Scenario: Geração com Auto ou sem chave
- **WHEN** o modelo de notas salvo é `open-software/auto`, ou não há chave OpenRouter
- **THEN** com Auto o app troca, uma vez, pelo modelo padrão explícito e a geração vai direto à OpenRouter; sem chave a geração falha com `openrouter_api_key_missing` sem requisição
- Test: unit (Rust) — carga de settings com Auto; integration — sem chave, zero requisições

### Requirement: Falha clara sem chave nem servidor
Quando falta a chave do provedor de que a nota precisa, a nota SHALL dizer, na língua da interface (pt-BR ou en), qual chave falta e onde salvá-la (Configurações > Modelos > Mais opções > Chaves de API de provedores), sem tentar outra rota, e MUST NOT mostrar o erro cru da biblioteca HTTP. A chave Groq só é exigida com o motor de transcrição Groq; com o motor Local, a falha de transcrição é a do motor local. O áudio fica salvo e "Tentar de novo" continua disponível. Sem a chave OpenRouter, a limpeza da transcrição é pulada e a nota segue.

#### Scenario: Transcrição sem chave Groq e sem servidor
- **WHEN** o motor de transcrição é Groq, não há chave Groq e uma gravação termina
- **THEN** a transcrição falha com `groq_api_key_missing` sem nenhuma requisição de rede, e a nota mostra traduzida a frase "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings."
- Test: integration — servidor sentinela em porta efêmera, injetada como a URL legada, recebe zero requisições; unit — `note-failure-banner.test.tsx` traduz a frase em pt-BR

#### Scenario: Motor Local sem chave Groq
- **WHEN** o motor de transcrição é Local, não há chave Groq e uma gravação termina
- **THEN** a transcrição é feita localmente e nenhuma mensagem de chave Groq aparece
- Test: integration — transcritor local falso e servidor sentinela com zero requisições

#### Scenario: Geração sem chave OpenRouter e sem servidor
- **WHEN** o motor de Notas é OpenRouter e não há chave OpenRouter
- **THEN** a geração falha com `openrouter_api_key_missing` e a nota mostra a frase de transcrição salva que pede a chave OpenRouter
- Test: integration — nenhuma requisição; unit — `processing.rs` mapeia o código para a frase; `note-failure-banner.test.tsx`

### Requirement: Limite de taxa da Groq na transcrição de nota
Na transcrição direta à Groq, um 429 SHALL virar espera pelo tempo que a Groq informa, e não falha da nota. As requisições de transcrição final MUST respeitar um ritmo comum às duas faixas. Uma espera longa SHALL pausar a nota com o progresso salvo, mostrar quando ela continua e retomar sozinha; os turnos prontos MUST NOT ser reenviados.

#### Scenario: 429 com retry-after vira espera
- **WHEN** a Groq responde 429 com `retry-after` de alguns segundos e depois 200
- **THEN** o erro do 429 leva `retryAfterMs`, a transcrição do turno espera esse tempo, repete a mesma requisição e o turno sai com o texto, sem falha na nota
- Test: integration — `clovy_api_notes_tests.rs` (Groq falsa com cabeçalhos) e `processing.rs` (transcritor falso, relógio pausado)

#### Scenario: Limite longo pausa e diz quando continua
- **WHEN** a Groq responde 429 com `retry-after` acima de 90 s
- **THEN** a rodada para com `transcription_rate_limited`, a nota mostra "O Clovy continua a transcrição às HH:MM", os turnos não iniciados ficam pendentes e o fallback da faixa não roda
- Test: integration — `processing.rs`, stream com transcritor falso

#### Scenario: Nova tentativa só envia o que falta
- **WHEN** uma rodada pausada é retomada
- **THEN** os turnos já transcritos são reaproveitados e só os pendentes vão à Groq
- Test: integration — `processing.rs`, duas rodadas no mesmo banco contando requisições

#### Scenario: Job longo da faixa inteira termina
- **WHEN** uma gravação audível vira um job da faixa inteira que passa do áudio da hora e pausa antes do último pedaço
- **THEN** cada pedaço pronto fica salvo, a retomada só envia os que faltam, e a nota termina
- Test: integration — `processing.rs`, oito pedaços de 30 s contra 100 s por hora, banco em memória

#### Scenario: A pausa termina pelo relógio de parede
- **WHEN** a nota pausou e o Mac dormiu além do prazo, ou o relógio voltou
- **THEN** a pausa termina na hora de parede marcada e nunca se estende por um relógio que voltou
- Test: unit — `processing.rs`, `RequestPacer` com relógio injetável

#### Scenario: Pausa de uma chave não segura outra rota
- **WHEN** a conta da chave A está pausada
- **THEN** uma chave B e a rota do Clovy API sem chave seguem mandando, e a impressão digital da chave nunca contém a chave
- Test: unit — `processing.rs`

#### Scenario: Retomada e "Tentar de novo" ao mesmo tempo
- **WHEN** a retomada automática e um "Tentar de novo" disputam a mesma nota pausada
- **THEN** só um reivindica a pausa e a nota é gerada uma vez só
- Test: integration — `processing.rs` (reivindicação no banco), unit — `processing_queue.rs` (revalidação ao pegar o lock)

#### Scenario: Troca de modelo entre a pausa e a retomada
- **WHEN** o modelo de transcrição muda entre a pausa e a retomada
- **THEN** nenhum turno nem pedaço do outro modelo é reaproveitado, e uma sessão do modelo padrão de antes mantém a identidade dos seus turnos
- Test: unit/integration — `processing.rs`

#### Scenario: Chave trocada no meio do run
- **WHEN** a chave Groq salva muda entre duas requisições
- **THEN** cada requisição lê a rota uma vez, e o pacer e a chamada usam a mesma chave
- Test: unit — `processing.rs`

#### Scenario: Chave salva que o cofre não devolve
- **WHEN** há chave Groq salva nas configurações, mas o cofre não a devolve
- **THEN** a transcrição falha com `provider_api_key_unavailable` e a nota diz, em pt-BR ou en, para salvar a chave de novo; nada vai ao Clovy API e a prévia não envia nada
- Test: unit — `processing.rs`, `src/test/note-failure-banner.test.tsx`

#### Scenario: Retry suspenso enquanto a retomada termina
- **WHEN** o retry manual é suspenso antes de mostrar o estado e a retomada termina a nota
- **THEN** o retry não grava `transcribing` por cima da nota `ready` e, pulado, deixa a nota como a retomada a deixou
- Test: integration — `processing.rs` (banco em memória e tickets reais da fila)

#### Scenario: Ritmo dentro do limite por minuto
- **WHEN** as duas faixas têm muitos turnos para transcrever
- **THEN** nunca começam mais de 18 requisições em 60 s, somando as faixas e a prévia
- Test: unit — `processing.rs`, `RequestPacer` com relógio pausado

#### Scenario: Áudio da hora dentro do limite
- **WHEN** o áudio cobrado na última hora já chegou a 7.200 s
- **THEN** a próxima requisição espera a janela liberar; se a espera passa de 90 s, a nota pausa com a hora de retomada
- Test: unit — `processing.rs`, `RequestPacer` com orçamento de áudio

#### Scenario: A prévia não gasta o áudio da nota
- **WHEN** a prévia ao vivo já gastou 3.600 s de áudio na última hora, ou não há requisição livre no minuto, ou um 429 pausou a conta
- **THEN** o trecho da prévia é descartado sem esperar, e a transcrição final mantém o resto do orçamento
- Test: unit — `processing.rs`, `try_acquire`

#### Scenario: Fala da faixa em pacotes, sem silêncio
- **WHEN** a chave Groq está salva e uma faixa tem vários turnos de fala
- **THEN** os turnos seguidos vão juntos numa requisição de até 600 s com tempos de segmento, sem o silêncio entre eles, e cada turno volta com seu texto e seu job
- Test: integration — `processing.rs`, stream com transcritor de pacote falso; `clovy_api_notes_tests.rs`, `verbose_json` com segmentos contra Groq falsa

#### Scenario: Pedaços abaixo do limite de upload
- **WHEN** um pacote ou um WAV longo vai à Groq
- **THEN** cada requisição tem no máximo 600 s (19,2 MB em PCM16 mono a 16 kHz), abaixo dos 25 MB, e o corte fixo repete 10 s no começo do pedaço seguinte, sem duplicar as palavras no texto
- Test: unit — `audio/turns.rs` e `processing.rs`

#### Scenario: Mensagem traduzida
- **WHEN** a interface está em pt-BR e a nota guarda a frase de limite ou de pausa da Groq
- **THEN** o banner mostra a frase em pt-BR, com a hora na de pausa
- Test: unit — `src/test/note-failure-banner.test.tsx`

### Requirement: Teste automático ao escolher provedor de Atividade
Ao escolher para Atividade uma CLI instalada, com modo one-shot e sem nível de saída estruturada medido, o app SHALL rodar o teste do provedor antes de recusar a escolha e SHALL aplicá-la se o nível medido suportar JSON schema. Falhas MUST ser explicadas pela causa real: login ausente aponta "Autenticar no Clovy"; cota ou prazo esgotado têm mensagem própria; só o nível insuficiente mostra a mensagem de JSON schema.

#### Scenario: Antigravity autenticado sem teste anterior
- **WHEN** o usuário escolhe Antigravity em Atividade e o perfil do Clovy está autenticado, sem nível medido
- **THEN** o app mostra que está testando, grava o nível medido e aplica a escolha

#### Scenario: Antigravity sem login no perfil do Clovy
- **WHEN** o teste automático falha por falta de autenticação
- **THEN** a escolha não é aplicada e a mensagem pede "Autenticar no Clovy", sem falar de JSON schema

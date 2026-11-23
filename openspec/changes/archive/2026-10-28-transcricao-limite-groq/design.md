# Design

## Context

A transcrição final de uma nota é feita por turno: a detecção por energia (`audio/turns.rs`) acha os trechos de fala de cada faixa, `coalesce_turns_for_transcription_avoiding_echo` junta turnos da mesma faixa separados por até 2,5 s, e cada turno vira um job durável (`note_transcription_jobs`). Silêncio já é pulado: a faixa de sistema em silêncio sai antes (`partition_silent_system_sources`), e cada pedaço passa por `source_is_effectively_silent` antes de ir à Groq. O stream (`transcribe_prepared_turn_stream`) roda até 2 tarefas ao mesmo tempo, uma por faixa.

Limites da Groq (docs `console.groq.com/docs/rate-limits` e `speech-to-text`, consultadas em 2026-10-27), iguais para `whisper-large-v3` e `whisper-large-v3-turbo`: 20 RPM, 2.000 RPD, 7.200 ASH (segundos de áudio por hora), 28.800 ASD. Mínimo cobrado de 10 s por requisição. Upload de até 25 MB no gratuito. Todo 429 traz `retry-after` em segundos; os `x-ratelimit-*` falam de RPD e TPM, não de áudio. O exemplo oficial de áudio longo (cookbook `tutorials/audio-chunking`) usa pedaços de 600 s com 10 s de sobreposição e espera 60 s num `RateLimitError`.

Medição da prévia ao vivo pelo código: ela vem ligada por padrão (`providers/mod.rs:1595`) e manda à mesma conta Groq cada trecho de 8 s com fala, das duas faixas (`audio/live_preview.rs:619`); cada trecho é cobrado como 10 s. Numa reunião de 1 h com o microfone falando ~30% e o sistema ~60% do tempo, a prévia gasta ~4.050 s de áudio e ~15 req/min; a transcrição final de turnos de 30 s pede outros ~3.500 s. Juntas passam dos 7.200 ASH: a final começa já estourada.

## Goals / Non-Goals

**Goals:**

- Um 429 vira espera, não falha da nota.
- Limite longo: pausa com progresso salvo, hora de retomada visível e retomada automática.
- Ritmo dentro de 20 RPM e 7.200 ASH somando as duas faixas e a prévia.
- Menos requisições e menos segundos cobrados por reunião.
- Mensagem traduzida.

**Non-Goals:**

- Mudar o Clovy API ou o caminho sem chave (sem chave, um turno por requisição, como antes).
- Ditado (fica fora do pacer; a folga de 2 RPM é dele).
- Ler o plano do usuário para acelerar além dos limites base.
- Mostrar a espera curta (até 90 s) na barra de progresso.
- Detecção de fala no modo só microfone (continua um job da faixa inteira, agora em pedaços de 600 s).

## Decisions

### D1. A espera vem do cabeçalho

`send_direct` lê, num 429, `retry-after` (segundos, inteiro ou decimal). Sem ele, usa `x-ratelimit-reset-requests` quando `x-ratelimit-remaining-requests` é `0`. O valor vai no erro como `details.retryAfterMs`, o mesmo campo do caminho pelo Clovy API. Até 2 s, `send_direct` espera e faz a nova tentativa de sempre; acima disso devolve o erro na hora, porque repetir antes do `retry-after` só gasta outra requisição. Sem cabeçalho, mantém a nova tentativa de 300 ms. Vale para todo `send_direct` (geração e limpeza na OpenRouter também).

### D2. Um pacer para o processo inteiro

`RequestPacer` em `processing.rs`: no máximo 18 inícios de requisição em 60 s e no máximo 7.200 s de áudio cobrado (duração do WAV, mínimo 10 s) em 60 min, mais um `paused_until` comum. Há um pacer por conta Groq, escolhido a cada requisição pela impressão digital da chave salva (8 primeiros bytes do SHA-256; a chave nunca é guardada), e um próprio para a rota do Clovy API sem chave, sem teto de requisições nem de áudio, parado só por um 429 que o servidor repasse (revisão G3). Cada pacer serve todas as notas, as duas faixas e a prévia daquela rota. Cada requisição lê a rota uma vez só (a chave salva, o modelo lido no início do run e o pacer dessa chave), e o cache de pedaços, o pacer e o transporte usam essa mesma rota, sem reler a chave (revisão H3). Só a ausência de chave salva vai para a rota do Clovy API: uma chave salva que o cofre não devolve faz a requisição falhar com `provider_api_key_unavailable`, a nota diz para salvá-la de novo e a prévia não envia nada (revisão I1). Os tempos são milissegundos do relógio de parede, injetável nos testes e protegido contra o relógio voltar; quem espera olha o relógio de novo a cada 30 s, então uma pausa termina na hora mesmo depois de o Mac dormir (revisão G2). Um 429 põe `paused_until` para todos (pelo `retry-after`, ou 60 s sem ele, como no exemplo da Groq) e a mesma requisição é repetida, até 5 vezes. A concorrência continua 2. Os testes injetam o próprio pacer e usam o relógio pausado do tokio.

Alternativa: ritmo adaptativo, rápido até o primeiro 429. Rejeitada: não há cabeçalho de RPM nem de ASH para áudio, e no plano base o primeiro estouro custa uma espera de até 60 s.

### D3. Espera longa pausa a nota

Uma espera acima de 90 s, seja a do pacer (orçamento de áudio da hora) ou a da Groq, vira `transcription_rate_limited` com `details.resumeAtMs`. Também vira pausa depois de 5 esperas seguidas. O turno ou pacote que a recebeu é gravado como falha (o job volta a pendente na próxima rodada), o stream para de lançar trabalho, e o fallback da faixa não roda. A nota fica com "Groq usage limit reached. Clovy continues transcribing at HH:MM. Parts already transcribed are kept." (hora local). `commands.rs` agenda a nova rodada para `resumeAtMs` + 5 s, conferindo o relógio a cada minuto (o sono do Mac para o relógio monotônico). Na hora, a retomada reivindica a pausa num único `UPDATE` (de `failed` com aquela frase para `transcribing`); um "Tentar de novo" que chegou antes, ou outra retomada, não reivindica nada. Quando uma rodada de nova tentativa pega o lock da nota, ela só roda se a nota ainda precisar. A fila conta as rodadas concluídas de cada nota, e o ticket guarda esse número no enqueue. Sob o lock, toda rodada revalida, seja qual for a profundidade na fila, porque a ordem de pegar o lock não é a de enfileirar. Uma nota `ready` nunca é processada de novo. Se outra rodada terminou depois do enqueue, o "Tentar de novo" só roda numa nota deixada em `failed`, e a retomada não roda. Sem nada entre o enqueue e o lock, o "Tentar de novo" de uma nota com falha roda, e a retomada roda na nota que sua reivindicação deixou em `transcribing`. Assim duas rodadas nunca geram a nota duas vezes (revisões G4 e H2). O "Tentar de novo" só se mostra na hora com um compare-and-set de `failed` para `transcribing`, nunca por cima de uma nota `ready` ou em processamento. Um retry que marcou a nota sempre roda, então uma rodada pulada nunca deixa a nota presa em `transcribing` (revisão I2). A nova rodada é a de "Tentar de novo": jobs prontos são reaproveitados e só os pendentes vão à Groq. Se o app fechar, a nota fica com a frase e o botão.

### D7. Pedaços prontos de um job dividido ficam salvos

Um job da faixa inteira (só microfone, fallback da faixa) ou um turno longo é dividido em pedaços de 600 s. Cada pedaço pronto vai para `note_transcription_chunks` (migração 51, em cascata com a sessão), com a chave do pedaço: o id de operação do pedaço (que carrega a impressão digital da entrada do job durável), a duração do áudio dele, o modelo e o idioma, para outra divisão ou outro modelo nunca reaproveitar um pedaço velho. A impressão digital da configuração do job também inclui o modelo. O modelo padrão não acrescenta nada a ela, então as sessões transcritas antes mantêm a identidade; trocar de modelo entre a pausa e a retomada transcreve a nota de novo, sem misturar dois modelos (revisão H1). A retomada lê os pedaços prontos e só envia os que faltam; um pedaço sem fala também fica salvo. Sem isso, duas horas audíveis pausavam antes do último pedaço em toda rodada e a nota nunca terminava (revisão G1).

### D4. Pedaços de 10 min e pacotes de turnos

`FINAL_TRANSCRIPTION_SPAN_MS = 600_000`. Escolha:

- 600 s é o pedaço do exemplo oficial da Groq; em WAV PCM16 mono a 16 kHz (32 KB/s) são 19,2 MB, abaixo dos 25 MB. FLAC não foi adotado: a Groq cobra por segundo de áudio, não por byte, e o projeto não tem codificador.
- O agrupamento de turnos da mesma faixa (intervalo de até 2,5 s, sem atravessar turno da outra faixa nem eco cortado) vai até 600 s.
- **Pacotes de turnos**, com a chave Groq: turnos seguidos da mesma faixa, prontos no stream, são concatenados com 0,5 s de silêncio entre eles, até 600 s, e vão numa requisição `verbose_json` com `timestamp_granularities[]=segment`. Cada segmento volta ao turno em que seu meio cai. O silêncio entre turnos nunca é enviado; o mínimo de 10 s é pago uma vez por pacote; cada turno mantém seu job durável, a atribuição por faixa e a ordem. Um turno que não coube, um job da faixa inteira e o caminho pelo Clovy API seguem um por requisição.
- O stream segura uma faixa enquanto o pacote não enche e ainda chegam turnos (até 256 turnos prontos); ao fim da preparação manda o que tiver.
- Um WAV maior que 600 s (job da faixa inteira, turno longo) é cortado em pedaços de 600 s com 10 s de sobreposição; as palavras repetidas no começo do pedaço seguinte saem quando os textos se juntam (a maior sequência de 2 a 40 palavras, sem caixa nem pontuação).

Ordem de grandeza: numa reunião de 1 h com 30% e 60% de fala, ~1.080 s e ~2.160 s de fala viram ~2 e ~4 pacotes, em vez de ~240 requisições de turnos de 30 s.

Compatibilidade: a id de cada job vem do intervalo do turno. Uma sessão que já tem turnos prontos no layout de 30 s calcula os dois layouts e fica com o que reaproveita mais turnos prontos; no empate, 600 s.

### D5. Mensagens

`notes.failure.groqUsageLimit` com o texto exato que o Rust grava para a Groq, e `notes.failure.rateLimitPaused` com `{time}`, reconhecida por padrão no banner. As duas em en e pt-BR.

### D6. A prévia cede o orçamento

A prévia passa pelo mesmo pacer com `try_acquire`: nunca espera, só manda quando há requisição livre no minuto, e só enquanto o áudio cobrado da hora fica dentro de 3.600 s. Fora disso o trecho é descartado (a ADR-0002 permite). Assim a transcrição final tem ao menos metade da hora ao terminar a reunião.

Alternativas: reaproveitar o texto da prévia na nota foi rejeitado, porque os cortes fixos de 8 s e a falta do prompt do dicionário dão um texto pior que o da passada por turnos, e os trechos não se alinham aos turnos. Desligar a prévia com a chave direta foi rejeitado por tirar do usuário uma função que ele ligou; o teto de 50% mantém a prévia quase sempre e protege a nota.

## Risks / Trade-offs

- Plano pago fica nos limites base (18 RPM, 7.200 ASH) até o Clovy ler o plano.
- Um pacote que falha reenvia todos os seus turnos na próxima rodada (até 600 s).
- Um segmento que atravessa a pausa entre dois turnos vai inteiro para um deles: o texto fica na faixa certa, mas pode trocar de turno vizinho.
- Os pedaços salvos de um job ficam até a sessão ser apagada (são só texto).
- A retomada automática não sobrevive ao fechamento do app.
- Em reuniões acima de ~1 h com muita fala, a nota pode pausar pelo ASH e terminar mais tarde, na hora mostrada.

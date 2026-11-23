# Design

## Context

Motivação e medições em `proposal.md`. Mapa do caminho atual (2026-10-17, `main` `a70e0149`):

- Atalho: helper Swift `june-dictation-helper` detecta fn por `flagsChanged` (`main.swift:848-850,1014-1035`) → stdout → Rust `ShortcutActivationController::handle_edge` (`dictation.rs:855-873`) → `start_listening` no stdin do helper (`dictation.rs:3245-3276`) → `start()` (`main.swift:1873-1934`): `AVCaptureDevice.requestAccess` a cada tomada, depois `RecordingCuePlayer.play(.start, completion: beginRecording)` (336 ms) e só então `AVAudioRecorder` AAC 44,1 kHz em arquivo (`main.swift:2092-2122`).
- Parada: `stop_listening` → arquivo `.m4a`; se o pico ficou abaixo de 0,04, `SNAudioFileAnalyzer` antes de `recording_ready`, com prazo de 5 s (`main.swift:357-410,2248-2268`).
- Transcrição: `dictate_transcribe` → Clovy API `/v1/dictate` com `x-groq-api-key` quando há BYOK (`clovy_api.rs:68,485-514`) → Groq `whisper-large-v3-turbo`. Cliente `reqwest` preguiçoso (`clovy_api.rs:3257-3266`).
- Limpeza: sempre roda e bloqueia a colagem (`dictation.rs:5373-5386,5652-5707`), em pedaços de 800 bytes em série (`dictation.rs:5731-5805`); motor por `LlmUsage::DictationCleanup` (`llm/registry.rs`), que hoje aceita CLI (perfil do dono: `cli: claude`) e recusa `None` (`registry.rs:398-405`).
- Colagem: `activationSettleDelay` 180 ms fixo quando o destino já está ativo (`main.swift:2573,2696-2698`).
- Referência de agilidade: liv-light (`~/Documents/Projetos/Desenvolvendo/liv-light`) chama Groq direto com `fetch` keep-alive, sem limpeza, cola sem espera.

## Goals / Non-Goals

**Goals:** metas de latência da spec `dictation`; um só caminho de limpeza (OpenRouter); nenhuma inicialização no primeiro aperto.

**Non-Goals:** ver `proposal.md`. Também fora: redesenhar o HUD do ditado e o seletor de microfone.

## Decisions

### D1. Limpeza: booleano + modelo OpenRouter

`DictationSettings` ganha `cleanupEnabled: bool` (padrão `false`) e `cleanupModel` (id de modelo OpenRouter, padrão o atual `DEFAULT_DICTATION_CLEANUP_MODEL`, `nvidia/nemotron-3-nano-30b-a3b`). `LlmUsage::DictationCleanup` deixa de aceitar `cli` e endpoint local na normalização e na gravação; a migração única (marcador gravado, como `llmCliDefaultApplied`) transforma `cli`/endpoint em `cleanupEnabled: false`. Para instalações sem CLI na limpeza, a migração também grava `cleanupEnabled: false`: o dono quer o comportamento do liv por padrão (decisão de 2026-10-17). O `style` (Standard/Casual/Formal) só vale com a limpeza ligada e fica escondido quando desligada.

Alternativa rejeitada: manter `ProviderRef` com `None` permitido. Mistura "qual motor" com "ligado/desligado" e deixa CLIs no seletor.

### D2. Rota direta com BYOK

Com chave Groq do usuário no armazenamento local de credenciais, `dictate_transcribe` chama `https://api.groq.com/openai/v1/audio/transcriptions` direto (mesmos parâmetros que a Clovy API usa hoje: modelo turbo, `language`, `prompt` do dicionário). Com chave OpenRouter do usuário, a limpeza chama `https://openrouter.ai/api/v1/chat/completions` direto com o mesmo prompt de sistema e `temperature: 0`. Sem chave, a rota pela Clovy API continua igual. O prompt de limpeza, hoje em `clovy-api/crates/.../prompts`, é copiado para o desktop como constante única usada pela rota direta (a Clovy API mantém o seu; contrato v1 intacto). Isso muda a regra de `AGENTS.md` "BYOK ... forwarded through Clovy API only": ADR nova registra que chave do usuário pode ir direto ao provedor dela; chave de serviço continua só na Clovy API.

Alternativa rejeitada: só aquecer a conexão com a Clovy API. Mantém um salto extra e o caminho do OS Accounts em produção; o liv prova que o direto basta.

### D3. Captura não espera o som

`start()` inicia a captura já no aperto (o `AVAudioRecorder`/sessão já preparado por D4) e toca o som em paralelo. Para o som não entrar no transcrito, o helper corta do arquivo o trecho em que o som tocou (offset conhecido, 336 ms) ou usa o processamento de voz do sistema para cancelar a saída; o construtor escolhe pelo resultado medido no transcrito e registra no design. `AVCaptureDevice.requestAccess` só é chamado quando o status não é `.authorized` (consulta síncrona de `authorizationStatus` no caso comum).

**Técnica escolhida (2026-10-17, construtor na box Linux): corte do trecho do som.** A captura começa no aperto e o som toca em seguida; o helper guarda quanto do início do arquivo o som cobre, medido no relógio do próprio arquivo (rodada 2, R1): no `AVAudioRecorder`, `currentTime` no instante em que o som toca + duração do som + 30 ms de margem; no microfone fixado (`SelectedDeviceRecorder`, cujo arquivo começa no primeiro sample), fim do som no relógio da sessão de captura menos o timestamp do primeiro sample gravado, + 30 ms, ou nada se o som acabou antes do primeiro sample e, ao parar, grava uma cópia sem esse trecho com `AVAssetExportSession` em `AVAssetExportPresetPassthrough` (sem recodificar o AAC) antes de `recording_ready`. Se o corte falhar, ou a tomada mal passar do som, segue o arquivo inteiro. Por quê: o cancelamento de eco do sistema exigiria trocar o `AVAudioRecorder` do caminho Auto-detect por `AVAudioEngine` com voice processing, o caminho que já regrediu nível de gravação antes (#86), e mexe no ganho e no timbre do que vai ao Whisper; o corte é determinístico, não altera o áudio que sobra e custa uma exportação sem recodificação na soltura. Pendências para o Mac (4.2): confirmar com áudio real que o som não aparece no transcrito, medir o custo do corte no intervalo `shortcut_key_up` → `paste_completed` e conferir nos dois gravadores que o corte não come o começo da fala. Se o corte custar caro ou falhar, a alternativa é voice processing.

### D4. Aquecimento na abertura

Na abertura do app e depois de cada tomada: o helper carrega os sons, consulta a permissão e prepara o gravador do próximo take (`prepareToRecord` ou sessão de captura pronta, sem gravar); o Rust lê o token/chave BYOK uma vez e mantém em memória, cria o cliente HTTP e abre a conexão com o destino da transcrição (requisição leve tipo `HEAD`/`GET /models` sem custo) e a renova se ficar ociosa por mais de 60 s antes de um aperto. Nenhum áudio é gravado fora de uma tomada.

### D5. "Nenhuma fala" depois do Whisper

O helper deixa de rodar `SNAudioFileAnalyzer` antes de `recording_ready`. Se o Whisper devolver texto vazio (ou só pontuação), o Rust emite `no_speech`. A heurística de nível de áudio continua só como dado de diagnóstico no log.

**Proteção contra alucinação no silêncio (rodada 2, R7, 2026-10-17).** O `SNAudioFileAnalyzer` também evitava colar o que o Whisper inventa sobre áudio quase vazio ("Thank you."). No lugar dele: (a) o helper manda a duração do arquivo em `recording_ready` (`audioDurationSeconds`); tomada com menos de 0,5 s não vai ao Whisper e vira "nenhuma fala" sem colar (sem duração, helper antigo ou Windows, segue o envio); (b) na rota direta ao Groq a transcrição pede `response_format=verbose_json` e descarta os segmentos com `no_speech_prob >= 0.6`; se nada sobrar, o texto fica vazio e vira `no_speech`. A rota pela Clovy API não muda (contrato v1).

### D6. Colagem sem espera fixa

Quando o destino já está ativo, cola sem `activationSettleDelay`; o polling de ativação (20 ms, até 1 s) continua para destinos que precisam ser ativados.

### D7. Limpeza em paralelo quando há pedaços

Com a limpeza ligada e texto acima de 800 bytes, os pedaços são limpos em paralelo (limite 4) e reunidos em ordem. Falha ou prazo estourado em qualquer pedaço cola o transcrito bruto inteiro (spec).

## Risks / Trade-offs

- [Som de início vazando no transcrito] → D3 exige verificação com áudio real e o construtor registra a técnica escolhida.
- [Chave BYOK saindo do desktop direto ao provedor] → é a chave do próprio usuário para o próprio provedor; teste garante que cada requisição leva só a chave do provedor chamado (já existe para a rota pela Clovy API).
- [Usuários que gostavam da limpeza perdem o embelezamento] → padrão desligado é decisão do dono; o controle fica visível em Configurações.
- [Aquecimento custa uma requisição ociosa] → leve e sem custo; só quando há atalho de ditado configurado.

## Migration Plan

Migração única no carregamento de `provider-settings.json`/`dictation-settings.json` com marcador; arquivos antigos não são apagados. Rollback: reverter o merge; a versão anterior lê `dictationCleanup` como estava (o campo não é apagado, só ignorado).

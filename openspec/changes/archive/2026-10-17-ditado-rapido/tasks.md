# Tasks

Gates de `AGENTS.md` ("Verification") em cada grupo para o que ele tocou. Medição de latência só no grupo 4.

## 1. Limpeza opcional e só OpenRouter

- [x] 1.1 `DictationSettings` com `cleanupEnabled` (padrão `false`) e `cleanupModel` (OpenRouter); registro recusa `cli` e endpoint para `DictationCleanup`; migração única com marcador (design D1). Verificar: testes unitários de normalização e de migração com fixtures (`cli: claude`, endpoint, Clovy) falhando antes e passando depois.
- [x] 1.2 Pipeline do ditado: com limpeza desligada não chama limpador; ligada, pedaços em paralelo (limite 4), falha ou prazo cola o bruto com aviso (D7). Verificar: testes de integração com transcritor e limpador falsos.
- [x] 1.3 Configurações: controle "Limpar texto do ditado" e escolha do modelo OpenRouter (estilo só aparece com limpeza ligada), i18n pt-BR/en. Verificar: teste de componente (controle desligado por padrão, seletor sem CLIs).

## 2. Rota direta com chave própria

- [x] 2.1 Transcrição direta ao Groq com BYOK e limpeza direta à OpenRouter com BYOK, cliente HTTP único aquecido; sem chave, Clovy API como antes (D2, D4 parte Rust). Verificar: testes de integração com servidores falsos contando requisições por destino e cabeçalhos com só a chave do provedor chamado.
- [x] 2.2 ADR nova (chave do usuário direto ao provedor), regra em `AGENTS.md` (Boundaries), `docs/llm-providers.md` e doc do ditado. Verificar: busca sem a regra antiga "forwarded through Clovy API only" fora de ADRs históricas.

## 3. Helper: início, aquecimento, fala e colagem

- [x] 3.1 Captura começa no aperto, som em paralelo sem vazar no transcrito, `requestAccess` só sem autorização (D3). Verificar: teste do helper ou do coordenador que a captura é iniciada antes do fim do som; registrar no design a técnica contra vazamento.
- [x] 3.2 Aquecimento na abertura e após cada tomada: sons carregados, gravador preparado, token e conexão prontos (D4). Verificar: teste do coordenador que a preparação acontece na abertura e não grava áudio.
- [x] 3.3 "Nenhuma fala" só depois do Whisper vazio; análise de fala da Apple fora do caminho de envio (D5). Verificar: teste com transcritor falso devolvendo vazio → `no_speech`, sem colar.
- [x] 3.4 Colagem sem espera fixa quando o destino já está ativo (D6). Verificar: teste do helper ou medição do intervalo `paste_target` → `paste_completed` < 50 ms com destino ativo.
- [x] 3.5 Indicador do ditado embaixo-centro por padrão, opção Em cima/Embaixo em Configurações, e causa raiz de hoje ele aparecer em y≈297 de 1200 em vez do topo (spec "Posição do indicador do ditado"). Verificar: testes unitários do cálculo da origem; frame da janela `Clovy Dictation` no Mac nas duas opções.

## 4. Prova

- [x] 4.1 `make verify` sem falhas.
- [x] 4.2 Medição no app de dev com chave Groq própria e limpeza desligada: 10 tomadas de 3-10 s; no `dictation-events.log`, `shortcut_key_down` → `listening_started` ≤ 150 ms (≤ 300 ms no primeiro aperto) e `shortcut_key_up` → `paste_completed` ≤ 1,5 s; mais 3 tomadas com limpeza ligada (OpenRouter) registrando o tempo. Evidência em `.tmp/verify/ditado-rapido/run-<id>/report.md` com a tabela antes/depois. Aceito pelo dono em 2026-10-17 com 3 tomadas (62-153 ms; 570-679 ms) e sem a medição com limpeza ligada (sem chave OpenRouter); evidência em `.tmp/verify/ditado-rapido/run-mac/report.md`.

## Workflow follow-up

- Revisão por harness diferente do autor, aceite do dono, `/openspec-archive-change`, `/sync` e `/cleanup`.

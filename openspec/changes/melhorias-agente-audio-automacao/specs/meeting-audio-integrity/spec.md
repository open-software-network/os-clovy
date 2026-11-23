# Spec Delta

## Purpose

Preservar a consistência das fontes e do tempo ativo da gravação entre áudio capturado, preview e transcrição definitiva, inclusive no Windows.

## ADDED Requirements

### Requirement: Controle de fontes conforme capacidade
Quando a captura System estiver disponível, o editor SHALL permitir escolher microfone ou microfone com sistema no Windows, exibindo o modo efetivamente solicitado e sua disponibilidade sem controles de permissão exclusivos do macOS.

#### Scenario: Alterar fontes no Windows
- **WHEN** System está pronto e o usuário escolhe microfone sozinho antes de gravar
- **THEN** somente Microphone é solicitado e os controles continuam acessíveis
- Test: component — plataforma Windows com readiness realista; native Windows — fontes solicitadas e WAVs sintéticos

### Requirement: Preview só sobrevive sem decisão definitiva
O app SHALL retirar previews cobertos por resultado definitivo ou descarte explícito do mesmo recording session/span/fonte e MUST preservar previews não cobertos ou cujo processamento final falhou. A ausência de texto final por si só MUST NOT significar descarte.

#### Scenario: Eco descartado
- **WHEN** Microphone tem preview e o processamento elimina explicitamente seu span por eco de System
- **THEN** o preview Microphone não reaparece ao lado do System final
- Test: regression — módulo real de reconciliação e evento terminal de descarte

#### Scenario: Falha ou outra sessão
- **WHEN** o processamento falha ou a decisão pertence a outra sessão/intervalo
- **THEN** o preview útil não é removido
- Test: unit — falha, cobertura parcial e duas sessões

### Requirement: Pausa não pertence ao áudio ativo
O WAV System no Windows SHALL excluir o tempo pausado também quando stop ocorre sem resume; silêncio real durante captura ativa MUST continuar representado.

#### Scenario: Stop durante pausa longa
- **WHEN** uma gravação sintética captura, pausa e encerra sem retomar
- **THEN** a duração do WAV exclui a pausa e corresponde ao tempo ativo da sessão
- Test: unit — relógio/contabilidade reais portáveis; native Windows — WASAPI pausa/stop e resume/silêncio

### Requirement: Diagnóstico sem causa presumida
O diagnóstico SHALL distinguir endpoint Console/Communications/saída da reunião, áudio de entrada, spans/processamento, ASR e preview. Não SHALL atribuir erro ao provedor ou à ausência do modelo local sem evidência, nem exportar conteúdo privado por padrão.

#### Scenario: Transcrição estranha sem áudio diagnóstico
- **WHEN** só existe o relato de trechos estranhos
- **THEN** a causa permanece não comprovada e os cenários sintéticos de cada fonte/preview são registrados separadamente das correções demonstradas
- Test: manual Windows — mic/System/duas fontes e preview on/off, sem dados privados

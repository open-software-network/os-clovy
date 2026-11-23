# Spec Delta

## Purpose

Transcrever áudio de ditado e notas com Groq/Whisper, preservando o fluxo local de gravação e os dados já existentes do usuário.

## ADDED Requirements

### Requirement: Groq Whisper para todo áudio novo
O sistema SHALL transcrever ditado e notas por Groq, com Whisper multilíngue válido e padrão whisper-large-v3-turbo, sem rota ou fallback Venice.

#### Scenario: Ditado com áudio real
- **WHEN** o usuário segura Fn, fala e solta com Groq configurado
- **THEN** a gravação é transcrita, a resposta identifica Groq e o texto é inserido no alvo segundo o fluxo existente
- Test: e2e — prova nativa pelo dono após grants e configuração, com eventos de gravação, transcrição e entrega; contrato local não substitui voz real.

#### Scenario: Nota e preview
- **WHEN** uma nota contém áudio válido e o usuário solicita transcrição ou preview autorizado
- **THEN** o sistema processa áudio/chunks/preview e persiste texto, idioma opcional e provider Groq sem perder a gravação ou os demais dados
- Test: integration — caminho de processamento/HTTP com erros e boundaries existentes; prova nativa de nota em perfil sandbox com configuração real.

### Requirement: Idioma e contexto de áudio
O sistema SHALL preservar o idioma opcional e o contexto/dicionário autorizados respeitando limites do provedor, sem enviar título da nota ou dados alheios. O desktop SHALL separar hint ASR limitado a termos inteiros e 200 bytes UTF-8, incluindo delimitadores, do contexto completo que continua na correção existente de notas e na única passada existente de cleanup de ditado no motor escolhido; se o cleanup falha, o texto bruto permanece. Não existe interruptor de cleanup no desktop atual. Esse envelope não é contador exato de tokens nem nova restrição do campo público `context`.

#### Scenario: Áudio em português com contexto
- **WHEN** o usuário escolhe português e fornece vocabulário no dicionário
- **THEN** Groq recebe idioma e hint apropriados sem corte de termo/UTF-8, a correção existente recebe contexto completo segundo o motor escolhido, e o resultado mantém texto e idioma opcionais sem novo contrato de segmentos ou palavras
- Test: integration — boundary multipart/parser com idioma/hint e ausência de título, casos multibyte/delimitadores/termo que não cabe e contexto completo na correção real; prova externa autorizada com vocabulário do dono.

### Requirement: Erro de ASR observável
O sistema SHALL informar falta de configuração, modelo inválido, rejeição do provedor, áudio sem fala e falha de transcrição sem fingir sucesso ou culpar permissões concedidas.

#### Scenario: Chave ausente ou inválida
- **WHEN** não existe credencial Groq válida ou o provedor recusa a credencial
- **THEN** o fluxo termina com erro útil sem transcrição/paste falsos, sem vazar segredo e sem pedir novamente TCC para mascarar a falha
- Test: integration — erros HTTP e ausência de configuração exercitados sem chamada paga; UI nativa mostra o erro correspondente.

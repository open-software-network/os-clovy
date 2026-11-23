# Spec Delta

## ADDED Requirements

### Requirement: Seleção explícita das plataformas publicadas
A distribuição SHALL aceitar all ou macos na variável não secreta CLOVY_RELEASE_PLATFORMS; ausente ou vazia SHALL selecionar all. A seleção MUST ser validada antes dos builds e usada de forma consistente até o manifesto. Valor diferente MUST falhar sem build/publicação. O modo all MUST continuar exigindo ambas as plataformas; macos MUST pular o job Windows inteiro.

#### Scenario: Seleção padrão
- **WHEN** a variável não existe ou está vazia
- **THEN** Mac e Windows são exigidos, sem reduzir silenciosamente a publicação
- Test: unit — seleção no shell real e contrato all do gerador

#### Scenario: Seleção Mac preservada durante a execução
- **WHEN** macos foi validado e a variável muda depois
- **THEN** os jobs e o manifesto continuam usando a seleção capturada para este run
- Test: análise da condição/outputs e execução GitHub Mac-only

#### Scenario: Seleção desconhecida
- **WHEN** o valor é diferente de all ou macos, respeitando maiúsculas e espaços
- **THEN** a execução falha antes dos builds e não publica manifesto
- Test: shell real do seletor e erro de modo inválido no CLI

## MODIFIED Requirements

### Requirement: Pacotes publicados
No modo all, o release SHALL conter DMG, app.tar.gz e sig universais, NSIS com sig e latest.json com as três plataformas. No modo macos, SHALL conter só os três artefatos Mac e o manifesto com as duas entradas Darwin. URLs e assinaturas MUST corresponder a esta tag. Só builds selecionados concluídos com sucesso SHALL permitir publicar; falha, cancelamento ou artefato exigido ausente MUST impedir publicação.

#### Scenario: Os dois builds passam
- **WHEN** all está selecionado e os jobs do Mac e do Windows terminam com sucesso
- **THEN** o release vX.Y.Z aparece como mais recente com os seis arquivos, e releases/latest/download/latest.json responde sem autenticação
- Test: integration — release real existente e teste do gerador para all

#### Scenario: Um build falha
- **WHEN** all está selecionado e o job do Windows falha
- **THEN** nenhum release aparece para a tag, nem com pacotes parciais; a tag pode ser reenviada depois de corrigir
- Test: análise dos resultados exigidos na condição de publicação; actionlint

#### Scenario: Publicação somente para Mac
- **WHEN** macos está selecionado, Mac termina com sucesso e Windows é pulado
- **THEN** o release contém DMG universal, Clovy.app.tar.gz, seu sig e latest.json apenas com darwin-aarch64 e darwin-x86_64; nenhum instalador ou entrada Windows é fabricado ou reaproveitado de versão antiga
- Test: unit — gerador Mac sem Windows; integration — run42314751511 aprovado

#### Scenario: Artefato selecionado ausente
- **WHEN** falta pacote, DMG ou assinatura exigida pelo modo escolhido
- **THEN** nenhum manifesto parcial é gravado e a publicação falha
- Test: unit — falta dos três artefatos Mac e falta de instalador/sig Windows em all

#### Scenario: Execução cancelada
- **WHEN** a execução ou um job obrigatório é cancelado
- **THEN** nenhum release é publicado, mesmo com artefatos de outro job disponíveis
- Test: análise da condição real incluindo cancelamento, sem alegar simulação do scheduler

### Requirement: Assinatura dos pacotes
Os pacotes de atualização SHALL ser assinados com a chave privada do updater do dono, lida só dos secrets do repositório; a chave MUST NOT aparecer em log, artefato ou arquivo do repositório. O build do Mac SHALL usar assinatura ad hoc quando não houver secrets de identidade Apple, e SHALL assinar com a identidade e notarizar quando os secrets de identidade e de notarização existirem. O instalador Windows selecionado SHALL ter Authenticode válido; macos MUST NOT exigir credenciais Windows.

#### Scenario: Sem secrets Apple
- **WHEN** macos está selecionado e o repositório só tem os secrets do updater
- **THEN** o build do Mac sai com assinatura ad hoc, sem notarização, e o release é publicado
- Test: integration — publicação Mac-only0.1.6; não qualifica Windows

#### Scenario: Com secrets Apple
- **WHEN** os secrets de identidade e de notarização da Apple existem
- **THEN** o app, os auxiliares e o `.dmg` saem assinados por essa identidade e o `.dmg` é notarizado e grampeado
- Test: unit — teste do script de build do Mac decide entre ad hoc e identidade pelas variáveis presentes (sem executar `codesign`)

# Spec Delta

## Purpose

Identificar de forma persistente e acessível o aplicativo que receberá o ditado, refletindo o candidato atual e o destino fixado ao parar.

## ADDED Requirements

### Requirement: Candidato e destino fixado
O HUD SHALL mostrar o ícone local e nome do candidato durante captura e manter o destino fixado antes do teardown durante processamento, colagem e erro da tomada.

#### Scenario: Troca durante e depois da captura
- **WHEN** a tomada começa em A, muda para B e para, depois o foco muda para C
- **THEN** o indicador acompanha A e B durante captura e permanece B até terminar, sem antecipar o pin
- Test: src/test/hud-target.test.ts; prova nativa pelo Main conforme docs/development.md

### Requirement: Isolamento de tomadas
O indicador SHALL rejeitar eventos e resoluções de ícone obsoletos, respeitar cancelamento e preserveActiveTake e limpar metadados ao terminar ou iniciar outra tomada.

#### Scenario: Ícone atrasado e reinício
- **WHEN** A resolve depois de B ou depois de cancelar e iniciar outra tomada
- **THEN** A não reaparece e a nova tomada não herda destino anterior
- Test: src/test/hud-target.test.ts

### Requirement: Fallback e compatibilidade
O HUD SHALL apresentar fallback genérico honesto quando destino ou ícone estiver indisponível, com tooltip e descrição EN/pt-BR, sem impedir controles ou vazar indicador para outros modos.

#### Scenario: Ícone falha ou evento antigo
- **WHEN** o ícone falha ou o evento não contém metadados
- **THEN** não há imagem quebrada nem destino inventado, e cancelamento continua disponível
- Test: src/test/hud-target.test.ts; smoke de navegador com bridge controlada

#### Scenario: Assinatura tardia durante colagem
- **WHEN** o HUD recebe final_transcript sem target antes do snapshot inicial e depois paste_target com B
- **THEN** o indicador hidrata B sem aceitar foco C durante processamento ou colagem, preservando um destino já fixado
- Test: src/test/hud-target-hydration.test.ts

#### Scenario: Destino fecha após pin
- **WHEN** o app fixado fecha antes da colagem
- **THEN** o indicador mantém a identidade fixada no erro e o texto nunca é redirecionado a outro app
- Test: src/test/hud-target.test.ts (apresentação); prova nativa pelo Main (colagem)

## Purpose

Define como o Clovy de release encontra, mostra, verifica e instala uma versão nova publicada pelo dono, e quando não verifica.

## ADDED Requirements

### Requirement: Fonte única e verificada de atualizações
O build de release SHALL buscar atualizações só no manifesto `latest.json` do release mais recente (não prerelease) de `guilhermexp/os-clovy`, num único canal estável. O app MUST instalar um pacote só quando a assinatura dele for válida para a chave pública do updater embutida no app; pacote sem assinatura ou com assinatura de outra chave MUST ser recusado sem alterar a instalação.

#### Scenario: Atualização válida
- **WHEN** o manifesto anuncia uma versão maior que a instalada, com pacote assinado pela chave do dono
- **THEN** o app oferece a versão nova
- Test: integration — prova no app de release instalado com a versão anterior

#### Scenario: Assinatura inválida
- **WHEN** o pacote anunciado não confere com a chave pública do app
- **THEN** a instalação falha com mensagem de erro e a versão instalada continua funcionando
- Test: unit — configuração do updater com a chave pública do dono e endpoint único; o plugin recusa assinatura de outra chave (prova manual com manifesto local adulterado)

#### Scenario: Sem versão nova
- **WHEN** o manifesto anuncia a mesma versão ou uma menor
- **THEN** o app não mostra aviso e, na verificação manual, informa que está atualizado
- Test: unit — decisão de oferecer atualização por comparação de versão

### Requirement: Quando o app verifica
O build de release SHALL verificar ao abrir e quando o dono pede em Ajustes → Geral. Uma falha de rede ou de manifesto MUST NOT bloquear o app nem mostrar erro na verificação automática; na verificação manual o erro SHALL aparecer. O build dev MUST NOT verificar nem oferecer atualização.

#### Scenario: Build dev
- **WHEN** o app dev abre ou o dono abre Ajustes → Geral
- **THEN** nenhuma requisição ao manifesto acontece e a ação de verificar não aparece
- Test: unit — `updates_are_disabled_in_dev_builds` e teste de UI de Ajustes

#### Scenario: Sem rede na abertura
- **WHEN** o app de release abre sem rede
- **THEN** o app abre normalmente, sem aviso de erro
- Test: unit — falha da verificação automática é silenciosa

### Requirement: Instalação só com o aceite do dono
O app SHALL mostrar a versão disponível e só baixar e instalar quando o dono aceitar. Depois de instalar, o app SHALL reiniciar na versão nova. Se houver gravação de reunião ou ditado em andamento, o app MUST NOT reiniciar até eles terminarem ou o dono escolher reiniciar depois.

#### Scenario: Dono adia
- **WHEN** o dono escolhe "depois"
- **THEN** nada é baixado e o aviso some até a próxima abertura ou verificação manual
- Test: unit — estado do aviso de atualização

#### Scenario: Gravação em andamento
- **WHEN** o dono aceita a atualização durante uma gravação
- **THEN** o app avisa que reinicia ao fim da gravação, sem cortar o áudio
- Test: unit — reinício adiado enquanto há gravação ou ditado ativo

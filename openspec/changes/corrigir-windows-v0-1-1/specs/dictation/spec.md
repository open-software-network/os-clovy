## ADDED Requirements

### Requirement: Atalhos do ditado no Windows
No Windows, o usuário SHALL poder salvar como atalho uma combinação de Ctrl/Alt/Shift/Win com uma tecla, ou uma tecla sozinha que o Windows enxerga: Ctrl, Alt ou Shift de qualquer lado, F13–F24, Pause, Scroll Lock, Insert e afins. O atalho padrão do ditado no Windows SHALL ser Ctrl direito. Teclas que o Windows não enxerga (como Fn na maioria dos teclados) não podem ser atalho, e a tela MUST dizer isso. Todo erro de validação ou de registro do atalho MUST aparecer na tela e continuar visível até a próxima ação do usuário.

#### Scenario: Combinação personalizada
- **WHEN** o usuário grava Ctrl+Shift+D para o ditado no Windows
- **THEN** o atalho é salvo, registrado e passa a iniciar o ditado
- Test: unit — validação e persistência; manual — Windows do dono

#### Scenario: Modificadora de qualquer lado
- **WHEN** o usuário aperta só Alt esquerdo (ou Ctrl esquerdo, ou Shift) durante a captura
- **THEN** essa tecla é gravada como atalho
- Test: unit — captura e validação aceitam `AltLeft`, `ControlLeft`, `ShiftLeft` e os do lado direito

#### Scenario: Tecla invisível ao Windows
- **WHEN** o usuário aperta só Fn durante a captura e nenhuma tecla chega ao Windows
- **THEN** ao fim da captura a tela explica que essa tecla é tratada pelo teclado e o Windows não a vê, e sugere Ctrl direito
- Test: unit — fim de captura sem tecla mostra a mensagem

#### Scenario: Erro de registro visível
- **WHEN** o Windows recusa o atalho porque outro app já o usa
- **THEN** a mensagem aparece na linha do atalho e não some com o fim da captura
- Test: unit — eventos de cancelamento não apagam o erro

### Requirement: Disparo de modificadora sozinha no Windows
Uma modificadora gravada como atalho sozinha SHALL disparar o ditado com o mesmo gesto do Fn no Mac: segurar sozinha grava enquanto segura; o gesto de alternância do Mac (dois toques) liga e desliga. O uso normal da tecla MUST continuar funcionando: apertar a modificadora junto com outra tecla, clique ou roda do mouse MUST NOT iniciar gravação nem mostrar o indicador do ditado. Entradas injetadas por software MUST NOT contar como gesto do usuário.

#### Scenario: Segurar Ctrl sozinho
- **WHEN** o atalho é Ctrl (qualquer lado) e o usuário segura só Ctrl
- **THEN** o ditado começa enquanto ele segura e termina ao soltar, entregando o texto
- Test: unit — máquina de estados do gancho com sequências de teclas e tempos

#### Scenario: Atalho de uso comum
- **WHEN** o atalho é Ctrl e o usuário faz Ctrl+C, Ctrl+clique ou Ctrl+roda do mouse
- **THEN** a ação normal acontece e nenhum ditado nem indicador aparece
- Test: unit — máquina de estados com teclas e eventos de mouse

#### Scenario: Diagnóstico do gatilho
- **WHEN** o usuário aperta o atalho e o ditado não começa
- **THEN** o registro de eventos do ditado mostra o que o gancho viu e por que não disparou
- Test: unit — cada decisão do gancho gera uma linha de diagnóstico sem conteúdo digitado

### Requirement: Ícone do app de destino no Windows
No Windows, o indicador do ditado SHALL mostrar o nome e o ícone do app que vai receber o texto, como no Mac, a partir da janela fixada no início do ditado. Quando o app não puder ser identificado, o indicador mostra o ícone genérico.

#### Scenario: Ditado no Bloco de Notas
- **WHEN** o usuário inicia o ditado com o Bloco de Notas em primeiro plano
- **THEN** o indicador mostra "Bloco de Notas" com o ícone dele, do início até a colagem
- Test: unit — evento de início e de colagem do Windows trazem o destino com identificador do app; manual — Windows do dono

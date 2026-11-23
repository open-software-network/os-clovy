# Spec Delta

## ADDED Requirements

### Requirement: Destino do ditado fixado no acionamento do atalho
No macOS, ao acionar o atalho de ditado, o Clovy SHALL capturar o destino da tomada: o app externo da frente e, quando a Acessibilidade permitir, sua janela e o campo focados. Esse destino SHALL valer para a fala, a finalização, a limpeza e a colagem automática da tomada. Ativação, abertura ou clique em outra janela durante a tomada MUST NOT alterar o destino nem o ícone do indicador. O stop MUST NOT fixar o destino de novo.

#### Scenario: Outra janela abre durante a fala
- **WHEN** o usuário aperta o atalho num campo do app A e, com o atalho pressionado, outra janela de outro app passa para a frente
- **THEN** o indicador continua mostrando o app A e, ao soltar, o texto é colado no campo original do app A

#### Scenario: Outra janela do mesmo app
- **WHEN** a tomada começou numa janela do app A e outra janela do próprio app A passa para a frente durante a fala
- **THEN** a colagem traz a janela original para a frente e cola nela, não na janela nova

#### Scenario: Nenhuma trava fora da tomada
- **WHEN** nenhuma tomada de ditado está ativa e o usuário troca de janela
- **THEN** o candidato a destino acompanha o foco normalmente, e a próxima tomada captura o destino em vigor no seu acionamento

#### Scenario: Tomada termina ou é cancelada
- **WHEN** a tomada é entregue, cancelada, rejeitada ou fica sem fala
- **THEN** o destino fixado é descartado e não é reutilizado por outra tomada

### Requirement: Destino da tomada indisponível não cola em outro lugar
Se o app, a janela ou o campo da tomada não existir mais ou não puder ser trazido para a frente até o prazo de ativação, o Clovy MUST NOT colar em outro app ou janela. O texto SHALL seguir a recuperação existente: permanecer no clipboard com o aviso atual e continuar disponível para o Control+V. Sem Acessibilidade para ler a janela, o destino SHALL se limitar ao app capturado no acionamento.

#### Scenario: Janela original fechou durante a fala
- **WHEN** a janela capturada no acionamento foi fechada antes da entrega
- **THEN** nada é colado, o aviso de destino indisponível é mostrado e o texto continua recuperável por Command+V e Control+V

#### Scenario: App não volta para a frente
- **WHEN** o app ou a janela da tomada não fica na frente até o prazo de ativação
- **THEN** o Command+V não é enviado e o texto segue a recuperação existente

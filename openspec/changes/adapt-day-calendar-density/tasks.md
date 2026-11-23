# Tasks

## 1. Escala e colunas

- [x] 1.1 Capturar RED comportamental da escala de viewport e das tarefas curtas sequenciais; entregar recibo `capture-red` e preservar a testemunha imutável até a associação pelo Main.
- [x] 1.2 Implementar cálculo adaptativo com medição estável da altura útil e migrar todos os consumidores; provar crescimento limitado, redimensionamento, dia longo/rolagem e ausência de NaN/ciclo de resize com GREEN e smoke visual.
- [x] 1.3 Calcular colunas a partir da escala e do retângulo efetivo, incluindo separação; provar coluna única para 09:00–09:10/09:30–09:40 com 600 px úteis e colunas legíveis para sobreposição real.
- [x] 1.4 Atualizar a documentação da escala/colunas em `docs/activity-timeline.md` e conferir coerência com a implementação e as provas.

## 2. Densidade dos cartões

- [x] 2.1 Implementar variantes compacta/intermediária/alta com título, metadados e notas progressivos, usando tokens existentes; provar que não aparecem linhas cortadas nos estados com título longo e notas.
- [x] 2.2 Preservar o nome acessível completo, foco/seleção, duração ativa e trilho de retomadas/legado; provar por teclado, ponteiro e controles existentes, sem alterar a lateral.
- [x] 2.3 Documentar o comportamento progressivo em `docs/activity-timeline.md` e verificar em pt-BR e EN, larguras estreita/larga e com o painel de seleção existente aberto/fechado.

## 3. Prova integrada

- [x] 3.1 Concluir regressões de navegação, busca, dias sem frentes, aproximação histórica e retomadas; registrar os resultados no snapshot final sem re-pinar testes de implementação ou wording incidental.
- [x] 3.2 Executar smoke no frontend real com dados sintéticos/IPC isolado, em pelo menos dois viewports, comparando antes/depois e registrando geometria/seleção e a lateral preservada.
- [x] 3.3 Concluir gate canônico, build de produção, detector Impeccable e verificador do worker; entregar logs/recibos, revisão atual, arquivos alterados e limites ao Main.

## Workflow follow-up

- Main associa RED, executa revisão externa em par, observa a superfície real e obtém `verify --record` em sessão independente antes do aceite.
- Main integra a entrega aprovada na `main`, atualiza o dev e o brain, e limpa caches/processos exclusivos preservando evidências. Sem push/deploy.

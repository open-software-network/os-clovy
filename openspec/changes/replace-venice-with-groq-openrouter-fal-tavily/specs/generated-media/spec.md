# Spec Delta

## Purpose

Preservar geração e edição de imagens e geração e animação de vídeos com endpoints fal reais até assets utilizáveis no Clovy.

## ADDED Requirements

### Requirement: Imagem e edição fal
O sistema SHALL gerar e editar imagens por modelos fal que suportem a ação, preservando conteúdo, proporção/orientação no preparo da edição, preferências de segurança e resultado utilizável sem lista de modelos Venice ou sucesso simulado. Entrada efetivamente enviada e saída pedida da edição SHALL ter até 1024x1024 pixels, com tamanho explícito e defensável para a estimativa de custo; o preparo geométrico não pode cortar/esticar conteúdo original para forçar saída quadrada.

#### Scenario: Gerar imagem
- **WHEN** o usuário escolhe um modelo fal disponível e solicita uma imagem
- **THEN** a fila é concluída e o asset final é validado e disponibilizado no fluxo existente com formato real e erro útil se a geração falhar
- Test: integration — submit/status/result/download e limites em sandbox; prova externa/UI requer chave e custo autorizados.

#### Scenario: Editar imagem
- **WHEN** o usuário fornece imagem e instrução de edição a um modelo fal que suporta edição
- **THEN** a imagem é enviada pelo contrato permitido, com proporção e orientação preservadas no preparo inclusive nas variantes suportadas, e a edição produz asset final sem upload público adicional não autorizado ou fallback Venice
- Test: integration — entrada de edição, geometria normal/extrema/orientação, payload/result e falhas com limites reais; prova externa em cópia sandbox autorizada.

#### Scenario: Reduzir foto grande sem cortar conteúdo
- **WHEN** a origem de uma edição ultrapassa o orçamento de 1024x1024 pixels
- **THEN** o sistema redimensiona a imagem inteira localmente, preserva proporção, orientação e bordas e envia no máximo esse orçamento; origem que cabe sem transformação é reutilizada, limites do decoder geram erro útil e nenhuma estimativa é apresentada como cobrança de conta comprovada
- Test: integration — fronteiras 1024x1024 e 1025x1024, PNG/JPEG grandes, marcas nas quatro bordas, EXIF/canvas e resultado com dimensão divergente; smoke runtime do preparo bloqueante sem chamada paga.

### Requirement: Vídeo e animação fal
O sistema SHALL executar geração de vídeo e animação de imagem por fal preservando os estados de job, parâmetros suportados, funding próprio/serviço e asset final, sem confundir status concluído com resultado válido. Poll BYOK SHALL exigir a chave fal atual sem fallback serviço e SHALL manter o job diante de erro recuperável de acesso; chave fal presente em poll de job pago pelo Clovy SHALL ser aceita e ignorada em favor da conta de serviço do job.

#### Scenario: Vídeo assíncrono
- **WHEN** uma geração de vídeo transita pela fila para execução e conclusão
- **THEN** o Clovy mantém o job, consulta seu estado e entrega o vídeo completo ou o erro final sem duplicar a geração
- Test: integration — transições queued/running/completed/error, BYOK/chave ausente ou de outra conta sem destruição indevida do job, funding e resultado/download, mais prova externa/UI autorizada.

#### Scenario: Retomar o mesmo vídeo após corrigir credencial
- **WHEN** um poll BYOK falha por chave ausente, indisponível ou sem acesso à conta do job e o usuário restaura uma chave válida para essa conta
- **THEN** o resultado recuperável mantém o `jobId` na conversa, a ferramenta existente pode continuar somente com esse identificador e entrega o asset do mesmo job sem novo submit, nova cotação ou fallback de funding; job indisponível continua terminal sem afirmar uma causa não demonstrada ou recriar vídeo automaticamente
- Test: integration — criação única, resultado de espera útil, restauração de credencial e retomada com zero submits adicionais; status terminal/cancelamento/404 indistinguível preservados, mais prova do caminho real do agente em sandbox.

#### Scenario: Animar imagem
- **WHEN** o usuário pede animação de uma imagem com modelo fal compatível
- **THEN** recebe o vídeo final com os parâmetros aceitos e não uma chamada de text-to-video disfarçada
- Test: integration — endpoint/input de animação distinto e job/result; prova externa em sandbox.

### Requirement: Limites e cobrança de mídia
O sistema SHALL preservar limites de entrada/download, cancelamento e erro pertinentes e SHALL usar preços/estimativas defensáveis dos endpoints selecionados, sem falsa cotação herdada de Venice.

#### Scenario: Falha ou asset excede limite
- **WHEN** fal retorna erro final, URL inválida ou asset maior que o limite vigente
- **THEN** o sistema não persiste asset inválido nem cobra uma conclusão fictícia e informa a falha sem vazar chave/URL sensível
- Test: integration — transições de erro, validação de URL/tamanho e ciclo de autorização/settlement existentes.

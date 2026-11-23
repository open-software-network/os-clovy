# media-generation Specification

## Purpose
Define como as ferramentas de mídia do Clovy (`generate_image`, `edit_image`, `generate_video`) geram e editam imagem e vídeo direto na fal, com a chave do próprio usuário, sem servidor intermediário.

## Requirements

### Requirement: Mídia direto na fal com chave própria
As ferramentas `generate_image`, `edit_image` e `generate_video` SHALL enviar o pedido direto à fal com a chave fal do usuário, aguardar o resultado na fila da fal, baixar o arquivo e gravá-lo como artefato da sessão como antes. A chave MUST ir só à fal, e nenhuma requisição MUST ir a outro servidor.

#### Scenario: Imagem gerada
- **WHEN** a chave fal está salva e um turno do Claude Code chama `generate_image` com um prompt
- **THEN** o pedido vai à fila da fal com o modelo escolhido em Configurações, e a imagem aparece como artefato da sessão
- Test: integration — fal falsa (envio, consulta de estado, resultado) e verificação do artefato gravado

#### Scenario: Vídeo demorado
- **WHEN** a fal mantém o pedido de vídeo na fila por vários minutos
- **THEN** a ferramenta consulta o estado até o fim ou até o tempo limite e devolve o vídeo ou erro de tempo esgotado
- Test: integration — fal falsa que responde "na fila" N vezes

### Requirement: Mídia sem chave fal
Sem a chave fal salva, as ferramentas de mídia SHALL devolver erro que diz que falta a chave fal e onde salvá-la, sem fazer requisição. A triagem local de conteúdo explícito por palavras MUST continuar antes do envio.

#### Scenario: Sem chave
- **WHEN** não há chave fal e `generate_video` é chamada
- **THEN** a ferramenta devolve o erro de chave fal ausente e a fal falsa não recebe nada
- Test: integration — fal falsa com contador zerado

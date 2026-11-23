# Spec Delta

## Purpose

Define o tema com que o Clovy abre e o fundo desenhado atrás das telas de conversa (Nova sessão e Início), incluindo a escolha do fundo pelo usuário e o comportamento do fundo animado.

## ADDED Requirements

### Requirement: Tema inicial escuro
Sem preferência de tema salva, o app SHALL abrir em modo escuro já no primeiro quadro, sem passar pelo claro. Uma preferência salva (claro, escuro ou sistema) MUST continuar valendo como antes.

#### Scenario: Primeira abertura
- **WHEN** o app abre sem a chave `os-clovy:theme` no armazenamento local, num sistema em modo claro
- **THEN** a janela principal abre com `data-theme="dark"` antes do primeiro paint e o seletor de tema em Aparência mostra "Escuro"
- Test: unit — `getStoredTheme`/`resolveTheme` sem chave salva; unit — script de `index.html` executado em jsdom sem chave

#### Scenario: Preferência salva
- **WHEN** o usuário salvou "Claro" e reabre o app
- **THEN** o app abre em modo claro
- Test: unit — leitura da preferência salva

### Requirement: Telas de conversa sem lavagem colorida
A Nova sessão, o Início e a etapa de personalidade do onboarding MUST NOT pintar gradiente na cor da marca atrás do conteúdo. Com o fundo "Nenhum", essas telas mostram só a superfície neutra do tema.

#### Scenario: Nova sessão com fundo Nenhum
- **WHEN** o usuário abre a Nova sessão em modo escuro com o fundo "Nenhum"
- **THEN** a área atrás de "No que vamos trabalhar?" e do composer tem a cor da superfície do tema, sem faixa azul ou verde-água em nenhuma altura
- Test: unit — regras CSS de `.agent-workspace` sem `--hero-wash` nem `--brand`; visual — capturas claro e escuro

### Requirement: Escolha do fundo
Configurações → Aparência SHALL oferecer a linha "Fundo" com as opções "Drift", "Imagem" e "Nenhum", com "Drift" como padrão. A escolha MUST ficar salva entre reinícios e MUST valer na hora, sem reiniciar o app.

#### Scenario: Trocar para Nenhum
- **WHEN** o usuário escolhe "Nenhum" com a Nova sessão aberta em outra aba
- **THEN** o desenho some da Nova sessão sem recarregar a janela, e continua sem desenho após reiniciar o app
- Test: unit — módulo de preferência (padrão, valor inválido, evento de mudança); component — fundo desmontado ao receber o evento

### Requirement: Fundo Drift
O fundo "Drift" SHALL desenhar o preset `drift` do asciify.org/backgrounds com os controles do dono: marca `dot`, tinta `#8f8d86`, velocidade 1, célula 11, opacidade sutil 0.14 (textura quase imperceptível nos dois temas), cena `flow` com contraste 1.4 e escala 1.6 para abrir vazios orgânicos móveis também no centro, rastro ao passar o ponteiro. O desenho MUST ficar atrás do conteúdo da Nova sessão e do Início, cobrindo toda a superfície de ponta a ponta, inclusive o centro atrás do título, do composer e das sugestões, mantendo texto e controles legíveis e clicáveis. O campo MUST alcançar as quatro bordas do painel, sem faixa vazia causada por padding do host, e ser recortado somente pelo raio do painel. Os vazios MUST vir do campo, não de máscara ou área de limpeza fixa. Véus do composer MUST ficar contidos no cartão e não cobrir a faixa inferior do painel. O desenho MUST NOT receber clique, foco nem leitor de tela.

#### Scenario: Conteúdo por cima do desenho
- **WHEN** a Nova sessão abre com "Drift"
- **THEN** o título, o composer e as sugestões ficam legíveis sobre o desenho contínuo, sem zona central fixa nem coluna livre, clique e teclado chegam a eles, e o desenho está marcado `aria-hidden`
- Test: component — kit simulado recebe `preset: drift`, `mark: 'dot'` sem a opção `clear` de coluna; visual — capturas claro e escuro, janela larga e janela estreita (390 px), incluindo as quatro bordas e crops laterais

### Requirement: Ciclo de vida do fundo
Cada tela SHALL ter no máximo um desenho do fundo ativo, destruído ao sair da tela ou ao trocar para "Nenhum" ou "Imagem". O desenho MUST pausar quando a janela está oculta ou o elemento fora da tela, MUST mostrar um quadro parado com movimento reduzido, e falha de carregamento ou falta de WebGL2 MUST deixar o fundo liso sem erro visível.

#### Scenario: Movimento reduzido
- **WHEN** o sistema pede movimento reduzido e a Nova sessão abre com "Drift"
- **THEN** o fundo mostra um quadro parado e não anima
- Test: visual — emulação de `prefers-reduced-motion: reduce`

#### Scenario: Falha ao carregar
- **WHEN** o módulo do fundo falha ao carregar ou o WebGL2 não está disponível
- **THEN** a Nova sessão funciona com fundo liso e nenhum erro aparece para o usuário
- Test: component — import do kit rejeitado; visual — WebGL desligado

#### Scenario: Limpeza
- **WHEN** o usuário sai da Nova sessão para uma nota e volta duas vezes
- **THEN** existe um único desenho ativo e os anteriores foram destruídos
- Test: component — `destroy` chamado uma vez por montagem; desmontagem antes do import terminar impede a criação do renderer

### Requirement: Fundo de imagem
"Imagem" SHALL abrir um diálogo nativo para escolher PNG, JPEG ou WebP local de até 20 MB. A validação MUST limitar cada dimensão a 8192 pixels e a alocação de decodificação a 128 MiB; excesso SHALL preservar a escolha anterior e mostrar o erro inline, sugerindo reduzir a resolução. O app MUST validar e copiar o arquivo para `backgrounds/` dentro de `app_paths::app_data_dir`, substituindo a cópia anterior somente após validação, sem referenciar o caminho original e sem upload. O fundo SHALL cobrir Nova sessão e Início com `object-fit: cover`, centralizado, chegando às quatro bordas e recortado pelo raio do painel, com `aria-hidden` e `pointer-events: none`. Um único véu na cor da superfície do tema, de intensidade constante 0.82, MUST preservar a legibilidade nos dois temas. A escolha `drift | image | none` MUST persistir em `os-clovy:background`, mantendo valores antigos válidos; metadados da cópia ficam no diretório do app. Só um renderer pode existir.

#### Scenario: Escolher imagem
- **WHEN** o usuário seleciona "Imagem" e escolhe um arquivo válido no diálogo nativo
- **THEN** a cópia local substitui a anterior, o nome aparece com "Escolher imagem" e "Remover", o Drift é destruído e o fundo muda sem reiniciar
- Test: unit — normalizador e comando copy/replace; component — estados de Aparência e ciclo de vida; visual — Nova sessão/Início, claro/escuro, largo/390 px

#### Scenario: Remover imagem
- **WHEN** o usuário aciona "Remover"
- **THEN** o app apaga sua cópia, preserva o original e salva "Nenhum"
- Test: unit — comando remove; component — preferência e controles após remoção

#### Scenario: Arquivo rejeitado ou diálogo cancelado
- **WHEN** o arquivo tem tipo inválido, excede 20 MB ou não pode ser lido/decodificado
- **THEN** Aparência mostra erro inline e mantém a escolha e a cópia anteriores; cancelar o diálogo também preserva a escolha, sem erro
- Test: unit — comandos rejeitam arquivos; component — erro/cancelamento preservam a preferência

#### Scenario: Cópia ausente ou corrompida
- **WHEN** a imagem salva não existe ou não pode ser decodificada
- **THEN** Nova sessão e Início mostram fundo liso sem erro, e Aparência informa imagem indisponível com ações de escolha e remoção
- Test: unit — cópia ausente/corrompida; component — fallback e estado indisponível; visual — arquivo ausente

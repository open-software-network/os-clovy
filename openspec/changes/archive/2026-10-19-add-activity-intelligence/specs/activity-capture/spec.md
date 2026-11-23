# Spec Delta

## Purpose

Registra continuamente, só como texto e com controles de privacidade, o que o usuário vê e faz no Mac, guardando tudo num banco local cifrado separado do banco principal do Clovy.

## ADDED Requirements

### Requirement: Captura opcional com permissões explícitas
A captura de atividade SHALL vir desligada. Ao ligá-la, o Clovy MUST mostrar o estado das permissões necessárias (Acessibilidade, Gravação de Tela e, para eventos de entrada, Monitoramento de Entrada) com atalhos para o painel correspondente dos Ajustes do Sistema, e MUST não capturar enquanto a permissão obrigatória faltar.

#### Scenario: Ligar sem permissão
- **WHEN** o usuário liga a captura sem ter concedido Acessibilidade e Gravação de Tela
- **THEN** a interface mostra as permissões faltantes com botões que abrem os Ajustes do Sistema e nenhum quadro é gravado
- Test: unit — máquina de estado da captura com verificador de permissões simulado

### Requirement: Somente texto, nunca pixels
O sistema SHALL extrair texto da janela em foco preferindo a árvore de acessibilidade, habilitando a acessibilidade de apps Chromium/Electron quando necessário, e SHALL usar OCR local do Apple Vision como alternativa. Imagens MUST ser processadas apenas em memória e nunca gravadas em disco. Cada quadro MUST registrar horário, app, título da janela, URL do navegador quando houver, texto e a origem do texto (acessibilidade ou OCR).

#### Scenario: Quadro capturado por acessibilidade
- **WHEN** a captura está ligada e o usuário está num editor que expõe a árvore de acessibilidade
- **THEN** em até 2 segundos um quadro é gravado com origem "acessibilidade", app e título corretos e nenhum arquivo de imagem é criado
- Test: e2e — captura real no app de desenvolvimento com leitura do banco pelo export de depuração

#### Scenario: Fallback de OCR
- **WHEN** a janela em foco não expõe texto pela acessibilidade
- **THEN** o quadro é gravado com origem "OCR" e o texto reconhecido localmente
- Test: integration — extrator com fonte de acessibilidade vazia e imagem de teste reconhecida pelo Vision

### Requirement: Cadência e monitores secundários
O sistema SHALL capturar a tela principal a cada 2 segundos e, quando a opção estiver ligada, amostrar a janela do topo de cada monitor secundário a cada ~10 segundos como contexto, sem dividir a sessão em foco.

#### Scenario: Monitor secundário desligado
- **WHEN** a opção de monitores secundários está desligada
- **THEN** nenhum registro de tela secundária é gravado
- Test: unit — agendador de ticks com a opção desligada

### Requirement: Eventos de entrada sem conteúdo digitado
O sistema SHALL registrar cliques, teclas, troca de app e foco de janela apenas com horário e app. O conteúdo de teclas e texto digitado MUST ser descartado antes de gravar. O texto do clipboard SHALL ser o único conteúdo guardado, truncado e com segredos (senhas, tokens, chaves) redigidos.

#### Scenario: Tecla sem conteúdo
- **WHEN** o usuário digita uma senha num campo qualquer
- **THEN** os eventos gravados contêm só tipo, horário e app, sem caracteres digitados
- Test: unit — normalizador de eventos descarta conteúdo de teclas

#### Scenario: Clipboard com token
- **WHEN** o usuário copia um texto contendo um token de API
- **THEN** o evento de clipboard é gravado com o token redigido
- Test: unit — redator de clipboard com amostras de segredos

### Requirement: Exclusões de privacidade
O sistema SHALL descartar, antes de gravar, quadros e eventos de apps da lista de apps ignorados (comparação exata sem diferenciar maiúsculas) e de URLs cujo domínio ou subdomínio esteja na lista de domínios ignorados. Janelas anônimas/privadas MUST ser ignoradas sem fallback para OCR. As janelas do próprio Clovy MUST ser excluídas. Com a opção de pausa em vídeo protegido ligada, a captura MUST pausar durante reprodução de vídeo com DRM. Fora do horário de trabalho configurado, a captura MUST pausar.

#### Scenario: Domínio ignorado
- **WHEN** `youtube.com` está na lista e o usuário abre `www.youtube.com`
- **THEN** nenhum quadro dessa aba é gravado
- Test: unit — filtro de domínios com subdomínios

#### Scenario: Aba anônima
- **WHEN** o usuário foca uma janela anônima do navegador
- **THEN** nenhum quadro é gravado, nem por OCR
- Test: unit — filtro com janela marcada como privada

#### Scenario: Fora do horário
- **WHEN** o horário de trabalho é seg–sex 09:00–18:00 e são 20:00 de uma terça
- **THEN** a captura está pausada com motivo "horário" e o menu da barra indica a pausa
- Test: unit — avaliação do horário com relógio simulado

### Requirement: Pausa manual e indicação de estado
O usuário SHALL poder pausar e retomar a captura pelo menu da barra e pelas configurações. O menu da barra MUST indicar se a captura está ativa, pausada (e por quê) ou desligada. Pausas MUST ficar registradas com o motivo para a linha do tempo.

#### Scenario: Pausar pelo menu
- **WHEN** o usuário escolhe "Pausar captura" no menu da barra
- **THEN** nenhum quadro é gravado até retomar e um registro de pausa manual é criado
- Test: integration — comando de pausa e verificação de ausência de quadros e do registro de pausa

### Requirement: Banco de atividade cifrado e separado
Os dados de atividade SHALL ficar num banco SQLite próprio, separado do banco principal do Clovy, cifrado com SQLCipher usando chave aleatória de 32 bytes guardada no Keychain (serviço distinto em builds de desenvolvimento). O arquivo MUST ser ilegível sem a chave. O banco principal do Clovy MUST continuar abrindo e funcionando sem alteração. Se a chave estiver ausente e o arquivo existir, o Clovy MUST avisar o usuário e oferecer recriar o banco (dados anteriores irrecuperáveis), sem apagar nada sozinho.

#### Scenario: Arquivo ilegível sem chave
- **WHEN** o arquivo do banco de atividade é aberto por um cliente SQLite sem a chave
- **THEN** a leitura falha como arquivo não reconhecido
- Test: integration — abertura do arquivo cifrado com SQLite comum

#### Scenario: Banco principal intacto
- **WHEN** o Clovy com a captura ligada inicia sobre um banco principal existente
- **THEN** notas, sessões e migrações do banco principal abrem e funcionam como antes
- Test: integration — suíte de migrações do banco principal sobre a build com SQLCipher

#### Scenario: Chave perdida
- **WHEN** o arquivo existe mas a chave não está no Keychain
- **THEN** a captura não inicia e a interface explica o problema e oferece recriar o banco
- Test: unit — inicialização com Keychain simulado vazio

### Requirement: Retenção
O sistema SHALL apagar quadros e eventos mais antigos que o período de retenção (padrão 30 dias, configurável), mas MUST apagar apenas o que já foi processado pela linha do tempo, e SHALL recuperar espaço em disco de forma incremental sem bloquear a captura. Pouco espaço em disco MUST pausar a captura com motivo registrado.

#### Scenario: Quadro não processado preservado
- **WHEN** existem quadros com mais de 30 dias ainda não processados
- **THEN** a rotina de retenção não os apaga
- Test: unit — retenção com cursor de processamento anterior aos quadros antigos

### Requirement: Export de depuração
Builds de desenvolvimento SHALL oferecer um comando que exporta o conteúdo do banco de atividade para inspeção local durante a verificação, sem expor a chave.

#### Scenario: Export em desenvolvimento
- **WHEN** o comando de export de depuração é executado num build de desenvolvimento
- **THEN** um arquivo legível com os quadros recentes é gerado no diretório de dados de desenvolvimento e a chave não aparece em saída alguma
- Test: integration — export sobre banco de teste e busca da chave na saída

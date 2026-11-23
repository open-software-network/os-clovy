# Spec Delta

## MODIFIED Requirements

### Requirement: Captura opcional com permissões explícitas
A captura de atividade SHALL exigir permissões de sistema apenas nas plataformas aplicáveis. No macOS, ao ligá-la ou estar ativa, o Clovy MUST mostrar o estado das permissões necessárias (Acessibilidade, Gravação de Tela e, para eventos de entrada, Monitoramento de Entrada) com atalhos para o painel correspondente dos Ajustes do Sistema, e MUST não capturar enquanto a permissão obrigatória faltar. No Windows, que não tem essas permissões, o Clovy MUST capturar sem pedir permissões e a interface MUST NOT mostrar pedidos de permissão.

#### Scenario: Ligar sem permissão
- **WHEN** o usuário liga a captura no macOS sem ter concedido Acessibilidade e Gravação de Tela
- **THEN** a interface mostra as permissões faltantes com botões que abrem os Ajustes do Sistema e nenhum quadro é gravado
- Test: unit — máquina de estado da captura com verificador de permissões simulado

#### Scenario: Ligar no Windows
- **WHEN** a captura está ligada no Windows
- **THEN** a captura opera sem pedir permissão e os ajustes de atividade não mostram linhas de permissão
- Test: unit — máquina de estado com permissões não aplicáveis; component — seção de atividade no Windows

### Requirement: Somente texto, nunca pixels
O sistema SHALL extrair texto da janela em foco preferindo a árvore de acessibilidade do sistema (Acessibilidade no macOS, UI Automation no Windows), habilitando a acessibilidade de apps Chromium/Electron quando necessário, e SHALL usar o OCR local do sistema (Apple Vision no macOS, Windows.Media.Ocr no Windows) como alternativa. Imagens MUST ser processadas apenas em memória e nunca gravadas em disco. Cada quadro MUST registrar horário, app, título da janela, URL do navegador quando houver, texto e a origem do texto (acessibilidade ou OCR).

#### Scenario: Quadro capturado por acessibilidade
- **WHEN** a captura está ligada e o usuário está num editor que expõe a árvore de acessibilidade
- **THEN** em até 2 segundos um quadro é gravado com origem "acessibilidade", app e título corretos e nenhum arquivo de imagem é criado
- Test: e2e — captura real no app de desenvolvimento com leitura do banco pelo export de depuração, no macOS e no Windows

#### Scenario: Fallback de OCR
- **WHEN** a janela em foco não expõe texto pela acessibilidade
- **THEN** o quadro é gravado com origem "OCR" e o texto reconhecido localmente
- Test: integration — extrator com fonte de acessibilidade vazia e imagem de teste reconhecida pelo OCR do sistema

### Requirement: Banco de atividade cifrado e separado
Os dados de atividade SHALL ficar num banco SQLite próprio, separado do banco principal do Clovy, cifrado com SQLCipher usando chave aleatória de 32 bytes guardada no cofre de credenciais do sistema (Keychain no macOS, Gerenciador de Credenciais no Windows), com serviço distinto em builds de desenvolvimento. O arquivo MUST ser ilegível sem a chave. O banco principal do Clovy MUST continuar abrindo e funcionando sem alteração. Se a chave estiver ausente e o arquivo existir, o Clovy MUST avisar o usuário e oferecer recriar o banco (dados anteriores irrecuperáveis), sem apagar nada sozinho.

#### Scenario: Arquivo ilegível sem chave
- **WHEN** o arquivo do banco de atividade é aberto por um cliente SQLite sem a chave
- **THEN** a leitura falha como arquivo não reconhecido
- Test: integration — abertura do arquivo cifrado com SQLite comum, no macOS e no Windows

#### Scenario: Banco principal intacto
- **WHEN** o Clovy com a captura ligada inicia sobre um banco principal existente
- **THEN** notas, sessões e migrações do banco principal abrem e funcionam como antes
- Test: integration — suíte de migrações do banco principal sobre a build com SQLCipher

#### Scenario: Chave perdida
- **WHEN** o arquivo existe mas a chave não está no cofre de credenciais
- **THEN** a captura não inicia e a interface explica o problema e oferece recriar o banco
- Test: unit — inicialização com cofre simulado vazio

## ADDED Requirements

### Requirement: Captura no Windows
No Windows, o Clovy SHALL capturar com a mesma cadência, exclusões, pausas, retenção e eventos de entrada sem conteúdo do macOS. O app de cada quadro MUST ser identificado pelo nome do executável, e janelas InPrivate ou anônimas MUST ser descartadas sem OCR. Em sistemas sem adaptador de captura, a vista "Hoje" MUST dizer que a captura não é suportada naquele sistema.

#### Scenario: Navegador no Windows
- **WHEN** a captura está ligada no Windows e o usuário lê uma página no Edge
- **THEN** o quadro registra o app Edge, o título, a URL da aba e o texto da página, sem arquivo de imagem
- Test: e2e — app de desenvolvimento na máquina Windows, leitura pelo export de depuração

#### Scenario: URL confirmada sem esquema inventado
- **WHEN** a página exibida no Edge usa HTTP e o controle de endereço omite o esquema
- **THEN** a URL registrada vem de metadados UIA do documento confirmado e preserva HTTP; se a identidade não for comprovada, fica desconhecida em vez de inventar HTTPS
- Test: unit — identidade UIA do documento; e2e — página HTTP própria e export de depuração no Windows

#### Scenario: Endereço em edição não muda a identidade da página
- **WHEN** uma página de domínio excluído continua exibida e o usuário digita outro endereço na barra sem navegar
- **THEN** o endereço em edição não substitui a identidade do documento e nenhum texto da página excluída é capturado
- Test: unit — metadados de documento e barra com valores divergentes; e2e — perfil de navegador próprio

#### Scenario: Janela InPrivate
- **WHEN** a janela em foco é uma janela InPrivate do Edge
- **THEN** nenhum quadro é gravado e nenhum OCR é feito
- Test: unit — filtro com descritor de janela privada; e2e — app de desenvolvimento na máquina Windows

#### Scenario: Cliques sem conteúdo
- **WHEN** o usuário clica e digita numa janela capturável no Windows
- **THEN** os eventos gravados contêm só tipo, contagem, horário e app
- Test: unit — normalização de eventos do adaptador Windows

#### Scenario: Sistema sem adaptador
- **WHEN** o Clovy roda num sistema sem adaptador de captura
- **THEN** a vista "Hoje" mostra que a captura não é suportada nesse sistema, sem banner de captura desligada
- Test: component — vista com disponibilidade "unsupported"

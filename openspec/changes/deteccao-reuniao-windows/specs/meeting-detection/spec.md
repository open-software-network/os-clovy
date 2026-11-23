## ADDED Requirements

### Requirement: Aviso de reunião pela posse do microfone
No macOS e no Windows, quando um app de chamada permitido passa a capturar o microfone e o Clovy não está gravando, o Clovy SHALL emitir `meeting-detection-event` do tipo `meeting_detected` com os rótulos legíveis dos apps, e o HUD SHALL perguntar se o usuário quer gravar. O sinal MUST ser só a posse do microfone, sem ler título de janela nem URL. Quando nenhum app permitido segura o microfone por duas sondagens seguidas, o Clovy SHALL emitir `meeting_cleared`.

#### Scenario: Google Meet no Chrome do Windows
- **WHEN** o Chrome abre o microfone para uma chamada do Meet no Windows e o Clovy não está gravando
- **THEN** o HUD recebe `meeting_detected` com o rótulo "Chrome"
- Test: unit — `meeting_detection::windows_probe` (parser com sessão ativa do serviço de áudio do Chrome levada ao `chrome.exe` principal) e máquina de estados com processo do Windows; CI Windows; manual — Windows de prova com binário de teste

#### Scenario: App de chamada empacotado
- **WHEN** o Teams novo (`ms-teams.exe`, hospedando WebView2) captura o microfone
- **THEN** o aviso mostra "Teams"
- Test: unit — subida de `msedgewebview2.exe` até `ms-teams.exe` no parser

#### Scenario: Rótulo legível
- **WHEN** o exe detectado é `msedge.exe`, `ms-teams.exe` ou um exe fora da lista
- **THEN** o rótulo é "Edge", "Teams" ou o nome sem `.exe`, nunca "exe"
- Test: unit — rótulos da lista do Windows

#### Scenario: App fora da lista
- **WHEN** um processo fora da lista de apps permitidos captura o microfone
- **THEN** nenhum aviso aparece
- Test: unit — filtro de apps permitidos com exe desconhecido

### Requirement: O próprio Clovy nunca dispara
Capturas do próprio processo do Clovy, do helper de ditado e de processos WebView2 hospedados pelo Clovy MUST NOT disparar o aviso nem contar como app de chamada no fim automático.

#### Scenario: Ditado no Windows
- **WHEN** o helper de ditado ou o Clovy captura o microfone no Windows
- **THEN** nenhum aviso aparece
- Test: unit — `owned_pids` excluídos depois da subida de pais no parser; manual — Windows de prova

### Requirement: Reforço pelo registro de consentimento no Windows
No Windows, a sonda SHALL usar como fonte principal as sessões de captura WASAPI ativas. Um app permitido que a enumeração não achou SHALL contar como em uso quando sua entrada em `CapabilityAccessManager\ConsentStore\microphone` (ou `NonPackaged`) tiver `LastUsedTimeStop == 0` e houver processo vivo com aquele exe. Entradas do registro de apps fora da lista MUST ser ignoradas.

#### Scenario: Navegador só no registro
- **WHEN** a enumeração WASAPI não traz o Firefox, mas `NonPackaged\C:#Program Files#Mozilla Firefox#firefox.exe` tem `LastUsedTimeStop == 0` e `firefox.exe` está rodando
- **THEN** a sonda devolve o Firefox com o pid do processo principal
- Test: unit — parser com entrada de registro

#### Scenario: Entrada antiga sem processo
- **WHEN** o registro diz que o Zoom está em uso, mas não há `zoom.exe` rodando
- **THEN** a sonda não devolve o Zoom
- Test: unit — parser com entrada de registro sem processo

### Requirement: Fim automático no Windows
No Windows, uma gravação iniciada pelo aviso de reunião SHALL seguir o mesmo fim automático do macOS: depois de 15 sondagens seguidas (15 s) sem nenhum app de origem segurando o microfone, abre a contagem visível de 15 s e, ao expirar, pede a finalização uma vez. O app voltar a segurar o microfone cancela a contagem. O comportamento no macOS MUST NOT mudar.

#### Scenario: Chamada encerrada no Windows
- **WHEN** a gravação foi iniciada pelo aviso com o Chrome e o Chrome solta o microfone por 15 s
- **THEN** a contagem abre e, ao expirar, um pedido de finalização fica retido até o renderer confirmar
- Test: unit — `MeetingEndTracker` com famílias do Windows vindas da sonda; manual — Windows de prova (fim observado pela sonda quando a captura de teste para)

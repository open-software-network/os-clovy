# Spec Delta

## Purpose

Define como o Clovy oferece, grava e guarda o vídeo da janela de uma reunião junto com a gravação de áudio, e como esse vídeo entra na geração da nota.

## ADDED Requirements

### Requirement: Oferta de vídeo da reunião
Ao iniciar uma gravação no macOS com uma janela de reunião aberta (Google Meet no navegador, Zoom ou Teams), o painel de opções da gravação e o aviso de reunião detectada SHALL oferecer "Gravar vídeo da reunião", desligado por padrão, com o nome do app e o título da janela. Sem janela de reunião, a opção MUST NOT aparecer.

#### Scenario: Google Meet aberto no Chrome
- **WHEN** o Chrome usa o microfone e tem uma janela com o Google Meet, e o usuário abre as opções da gravação
- **THEN** aparece "Gravar vídeo da reunião" com "Chrome" e o título da janela do Meet, desligado
- Test: unit (Rust) — escolha da janela a partir de uma lista de janelas e do app dono do microfone; component — opção visível só com janela detectada

#### Scenario: Sem reunião
- **WHEN** nenhuma janela aberta corresponde a uma reunião
- **THEN** o painel mostra só as opções de áudio
- Test: component — painel sem janela detectada

### Requirement: Permissão de gravação de tela
Ligar o vídeo sem a permissão de Gravação de tela SHALL pedir a permissão ou abrir o painel de Privacidade do sistema. Sem a permissão, a gravação MUST seguir só com áudio e a nota MUST registrar o motivo.

#### Scenario: Permissão negada
- **WHEN** o usuário liga o vídeo, a permissão de Gravação de tela está negada e ele inicia a gravação
- **THEN** o áudio grava normalmente, o vídeo não é gravado e a nota mostra que faltou a permissão
- Test: integration — helper de vídeo falso que informa permissão negada

### Requirement: Captura da janela junto com a sessão
Com o vídeo ligado, o sistema SHALL gravar só a janela escolhida, sem som, começando, pausando, retomando e terminando junto com a sessão de gravação. Falha ao iniciar ou durante o vídeo MUST NOT parar nem atrasar o áudio; se a janela fechar, o vídeo termina ali e o áudio continua.

#### Scenario: Pausa e retomada
- **WHEN** o usuário pausa e retoma uma gravação com vídeo
- **THEN** o vídeo não tem quadros do intervalo pausado e o início do vídeo fica registrado em relação ao início do áudio
- Test: integration — helper falso recebe pausa e retomada; registro do deslocamento

#### Scenario: Janela fechada no meio
- **WHEN** a janela do Meet fecha antes do fim da gravação
- **THEN** o vídeo fica salvo até aquele ponto e o áudio segue até o usuário terminar
- Test: integration — helper falso encerra cedo; sessão termina com áudio completo

### Requirement: Armazenamento do vídeo
O vídeo SHALL ficar na pasta da sessão de gravação, ao lado do áudio, registrado no banco como artefato da sessão. Apagar a nota ou a gravação MUST apagar o vídeo. Após um encerramento inesperado do app, o que já foi gravado MUST continuar reproduzível.

#### Scenario: Apagar a nota
- **WHEN** o usuário apaga uma nota gravada com vídeo
- **THEN** o arquivo de vídeo e o registro dele somem junto com o áudio
- Test: unit (Rust) — remoção da nota apaga o arquivo e a linha do artefato

#### Scenario: App encerrado no meio
- **WHEN** o app é encerrado à força durante uma gravação com vídeo e reaberto
- **THEN** a recuperação mantém o vídeo até o último trecho gravado e ele abre num reprodutor
- Test: integration — arquivo fragmentado truncado ainda lido pelo `ffprobe`

### Requirement: Nota com análise do vídeo
Com vídeo gravado e motor de Notas = `claude`, a geração da nota SHALL rodar a skill `video-analysis` sobre o vídeo da sessão, com a transcrição e o deslocamento entre vídeo e áudio, e a nota MUST incluir o que foi visto na tela (slides, telas compartilhadas, documentos) com o instante de cada item.

#### Scenario: Reunião com slides
- **WHEN** uma gravação com vídeo de uma apresentação termina e o motor de Notas é `claude`
- **THEN** a nota traz os pontos falados e o conteúdo dos slides visíveis, com instantes, e o resultado da geração registra que o vídeo foi analisado
- Test: integration — CLI falso que confere a concessão de ferramentas e o caminho do vídeo; manual — gravação real de uma call de teste

### Requirement: Nota sem análise do vídeo
Quando a sessão tem vídeo e o motor de Notas não é `claude`, a skill, o `python3` ou o `ffmpeg` não estão disponíveis, ou a análise falha ou estoura o tempo, a nota SHALL ser gerada só pela transcrição e MUST mostrar um aviso de que o vídeo não foi analisado, com o motivo.

#### Scenario: Motor de Notas que não é o Claude Code
- **WHEN** a sessão tem vídeo e o motor de Notas não é o CLI `claude` (por exemplo OpenRouter ou um endpoint)
- **THEN** a nota sai só pela transcrição e mostra "Vídeo não analisado: o motor de Notas atual não analisa vídeo"
- Test: unit (Rust) — escolha da rota de geração; component — aviso na nota em en e pt-BR

#### Scenario: Skill ausente
- **WHEN** o motor de Notas é `claude` e `~/.agents/skills/video-analysis` não existe
- **THEN** a nota sai pela transcrição com o aviso de skill ausente, sem iniciar o CLI com ferramentas
- Test: unit (Rust) — checagem de pré-requisitos com diretório temporário

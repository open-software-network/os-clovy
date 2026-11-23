import { defineMessages } from "../define";

// HUD windows: dictation + meeting-detection pill (hud.ts), agent sessions
// overlay (agent-hud.ts), and the recording pill (meeting-hud.ts).
export default defineMessages({
  en: {
    // Dictation HUD
    "hud.dictation.windowTitle": "Clovy Dictation",
    "hud.dictation.cancel": "Cancel dictation",
    "hud.dictation.stop": "Stop dictation",
    "hud.dictation.target": "Dictation destination: {app}",
    "hud.dictation.targetUnknown": "Dictation destination unidentified",
    "hud.status.idle": "Idle",
    "hud.status.listening": "Listening",
    "hud.status.transcribing": "Transcribing",
    "hud.status.stillTranscribing": "Still transcribing",
    "hud.status.pasting": "Pasting",
    "hud.status.pastingInto": "Pasting into {app}",
    "hud.status.pastingIntoUncleaned": "Pasting into {app} without cleanup",
    "hud.status.pastingIntoMissingOpenRouterKey":
      "Pasting into {app} without cleanup: OpenRouter API key missing.",
    "hud.status.previousApp": "previous app",
    "hud.status.dictationFailed": "Dictation failed.",
    // Dictation errors by stable code (src/lib/errors.ts); the native message
    // is only the fallback for codes without an entry.
    "hud.error.recordingStartBusy": "Dictation is already starting.",
    "hud.error.alreadyListening": "Dictation is already listening.",
    "hud.error.notListening": "Dictation is not listening.",
    "hud.error.microphonePermissionMissing": "Microphone permission is required.",
    "hud.error.audioStartFailed": "Could not start microphone recording.",
    "hud.error.missingRecording": "No recorded audio was available to transcribe.",
    "hud.error.emptyTranscript": "No transcript text was available to paste.",
    "hud.error.noSpeech": "No speech detected.",
    "hud.error.updateInstalling": "Clovy is installing an update and restarts in a moment.",
    "hud.error.pasteManually":
      "Clovy couldn't paste automatically. Your transcript is on the clipboard, so you can paste it with Cmd+V.",
    "hud.error.clipboardWriteFailed": "Could not write transcript to the clipboard.",
    "hud.error.recoveryCopyFailed":
      "Could not save or copy this dictation. The recording was kept for recovery.",
    "hud.error.recoveryClipboard":
      "Could not save this dictation. Use Cmd+V to keep the transcript.",
    "hud.error.recoveryUnavailable":
      "Clovy couldn't save this dictation. The recording was kept for recovery.",
    "hud.error.helperCommandInvalid": "The dictation helper rejected a command.",
    "hud.error.shortcutInvalid": "Shortcut configuration was invalid.",
    "hud.error.helperUnavailable": "Dictation helper process is not running.",
    "hud.error.responseInvalid":
      "Dictation returned a summary instead of dictated text. Try again.",
    "hud.error.audioWithoutText": "Dictation detected audio but produced no text. Try again.",
    "hud.error.providerNotConfigured":
      "Dictation requires a Groq transcription model through Clovy's model service.",
    "hud.error.cleanupTimeout": "Dictation cleanup timed out.",
    "hud.error.historySaveFailed": "Dictation history could not save this transcript.",
    "hud.error.recordingPathMissing": "Dictation helper did not provide a recording path.",
    "hud.error.hotkeyRegistrationFailed": "Could not register the dictation shortcut.",
    "hud.error.shortcutMonitorUnavailable": "Could not monitor global shortcut key events.",
    "hud.error.shortcutFnChordUnsupported":
      "The shortcut {shortcut} combines Fn with another key, which is no longer supported. Pick a different shortcut in Settings.",
    "hud.error.shortcutRegisterFailed": "Could not register the shortcut {shortcut}.",
    "hud.error.escapeCancelUnavailable": "Could not register Escape to cancel dictation.",
    "hud.error.cleanupInstruction": "Dictation cleanup returned an instruction response.",
    "hud.error.cleanupNoText": "Dictation cleanup response did not contain text output.",
    "hud.error.settingsUnavailable": "Settings lock failed.",
    "hud.error.settingsSaveFailed": "Could not save dictation settings.",
    "hud.error.shortcutIncomplete": "Shortcut is incomplete.",
    "hud.error.shortcutModifierRequired": "Shortcut must include at least one modifier key.",
    "hud.error.shortcutModifierRequiredWindows":
      "Shortcut must include at least one modifier key (Ctrl, Alt, Shift, Win).",
    "hud.error.shortcutKeyRequired": "Shortcut must include a supported non-modifier key.",
    "hud.error.shortcutDoublePressWindows": "Double-press shortcuts are not supported on Windows.",
    "hud.error.shortcutFnWindows": "The Fn key is not supported for Windows dictation shortcuts.",
    "hud.error.shortcutModifierOnlyWindows":
      "Modifier-only shortcuts are not supported on Windows.",
    "hud.error.shortcutPlatformUnsupported":
      "Dictation shortcuts are not supported on this platform.",
    "hud.error.shortcutDuplicate": "Push-to-talk and toggle dictation shortcuts must be different.",
    "hud.error.recordingUnsupported": "Dictation recording is not supported on this platform.",
    "hud.error.helperRestarting": "The dictation helper is restarting.",
    "hud.error.composerUnavailable": "Dictation could not deliver to this composer. Try again.",
    // Meeting detection prompt
    "hud.meeting.detected": "Meeting detected",
    "hud.meeting.record": "Record",
    "hud.meeting.dismiss": "Dismiss meeting prompt",
    "hud.meeting.micInUse": "Microphone in use",
    // Recording pill
    "hud.recording.windowTitle": "Clovy Recording",
    "hud.recording.recording": "Recording. Click to open Clovy",
    "hud.recording.paused": "Paused. Click to open Clovy",
    "hud.meetingEnd.title": "Meeting ended",
    "hud.meetingEnd.stop": "Stop now",
    "hud.meetingEnd.stopLabel": "Stop recording now",
    "hud.meetingEnd.keep": "Keep recording",
    "hud.meetingEnd.seconds": "{seconds}s",
    // Agent sessions HUD
    "hud.agents.windowTitle": "Clovy Agents",
    "hud.agents.activity": "Agent activity",
    "hud.agents.sessions": "Agent sessions",
    "hud.agents.hide": "Hide sessions HUD",
    "hud.agents.expand": "Expand agent activity",
    "hud.agents.collapse": "Collapse agent activity",
    "hud.agents.needInput": { one: "{count} needs input", other: "{count} need input" },
    "hud.agents.running": { one: "{count} running", other: "{count}" },
    "hud.agents.activeAgents": { one: "{count} active agent", other: "{count} active agents" },
    "hud.agents.status.thinking": "Thinking",
    "hud.agents.status.needsInput": "Needs input",
    "hud.agents.status.done": "Done",
    "hud.agents.status.failed": "Hit a problem",
    "hud.agents.status.stopped": "Stopped",
    "hud.agents.untitledSession": "Agent session",
  },
  "pt-BR": {
    "hud.dictation.windowTitle": "Ditado do Clovy",
    "hud.dictation.cancel": "Cancelar ditado",
    "hud.dictation.stop": "Parar ditado",
    "hud.dictation.target": "Destino do ditado: {app}",
    "hud.dictation.targetUnknown": "Destino do ditado não identificado",
    "hud.status.idle": "Inativo",
    "hud.status.listening": "Ouvindo",
    "hud.status.transcribing": "Transcrevendo",
    "hud.status.stillTranscribing": "Ainda transcrevendo",
    "hud.status.pasting": "Colando",
    "hud.status.pastingInto": "Colando em {app}",
    "hud.status.pastingIntoUncleaned": "Colando em {app} sem limpeza",
    "hud.status.pastingIntoMissingOpenRouterKey":
      "Colando em {app} sem limpeza: falta a chave de API da OpenRouter.",
    "hud.status.previousApp": "app anterior",
    "hud.status.dictationFailed": "Falha no ditado.",
    "hud.error.recordingStartBusy": "O ditado já está começando.",
    "hud.error.alreadyListening": "O ditado já está ouvindo.",
    "hud.error.notListening": "O ditado não está ouvindo.",
    "hud.error.microphonePermissionMissing": "É preciso permitir o acesso ao microfone.",
    "hud.error.audioStartFailed": "Não foi possível começar a gravar o microfone.",
    "hud.error.missingRecording": "Nenhum áudio gravado estava disponível para transcrever.",
    "hud.error.emptyTranscript": "Nenhum texto transcrito estava disponível para colar.",
    "hud.error.noSpeech": "Nenhuma fala detectada.",
    "hud.error.updateInstalling":
      "O Clovy está instalando uma atualização e reinicia em instantes.",
    "hud.error.pasteManually":
      "O Clovy não conseguiu colar automaticamente. Sua transcrição está na área de transferência, então você pode colar com Cmd+V.",
    "hud.error.clipboardWriteFailed":
      "Não foi possível copiar a transcrição para a área de transferência.",
    "hud.error.recoveryCopyFailed":
      "Não foi possível salvar nem copiar este ditado. A gravação foi guardada para recuperação.",
    "hud.error.recoveryClipboard":
      "Não foi possível salvar este ditado. Use Cmd+V para manter a transcrição.",
    "hud.error.recoveryUnavailable":
      "O Clovy não conseguiu salvar este ditado. A gravação foi guardada para recuperação.",
    "hud.error.helperCommandInvalid": "O assistente de ditado recusou um comando.",
    "hud.error.shortcutInvalid": "A configuração do atalho é inválida.",
    "hud.error.helperUnavailable": "O processo auxiliar de ditado não está em execução.",
    "hud.error.responseInvalid":
      "O ditado devolveu um resumo em vez do texto ditado. Tente de novo.",
    "hud.error.audioWithoutText": "O ditado detectou áudio, mas não gerou texto. Tente de novo.",
    "hud.error.providerNotConfigured":
      "O ditado precisa de um modelo de transcrição Groq pelo serviço de modelos do Clovy.",
    "hud.error.cleanupTimeout": "A revisão do ditado demorou demais.",
    "hud.error.historySaveFailed": "O histórico de ditado não conseguiu salvar esta transcrição.",
    "hud.error.recordingPathMissing": "O assistente de ditado não informou onde está a gravação.",
    "hud.error.hotkeyRegistrationFailed": "Não foi possível registrar o atalho de ditado.",
    "hud.error.shortcutMonitorUnavailable":
      "Não foi possível monitorar as teclas de atalho globais.",
    "hud.error.shortcutFnChordUnsupported":
      "O atalho {shortcut} combina Fn com outra tecla, o que não é mais suportado. Escolha outro atalho nos Ajustes.",
    "hud.error.shortcutRegisterFailed": "Não foi possível registrar o atalho {shortcut}.",
    "hud.error.escapeCancelUnavailable": "Não foi possível registrar Esc para cancelar o ditado.",
    "hud.error.cleanupInstruction": "A revisão do ditado devolveu uma instrução em vez do texto.",
    "hud.error.cleanupNoText": "A revisão do ditado não devolveu texto.",
    "hud.error.settingsUnavailable": "Não foi possível acessar os ajustes de ditado.",
    "hud.error.settingsSaveFailed": "Não foi possível salvar os ajustes de ditado.",
    "hud.error.shortcutIncomplete": "O atalho está incompleto.",
    "hud.error.shortcutModifierRequired":
      "O atalho precisa incluir pelo menos uma tecla modificadora.",
    "hud.error.shortcutModifierRequiredWindows":
      "O atalho precisa incluir pelo menos uma tecla modificadora (Ctrl, Alt, Shift, Win).",
    "hud.error.shortcutKeyRequired":
      "O atalho precisa incluir uma tecla suportada que não seja modificadora.",
    "hud.error.shortcutDoublePressWindows": "Atalhos de toque duplo não são suportados no Windows.",
    "hud.error.shortcutFnWindows": "A tecla Fn não é suportada nos atalhos de ditado do Windows.",
    "hud.error.shortcutModifierOnlyWindows":
      "Atalhos só com teclas modificadoras não são suportados no Windows.",
    "hud.error.shortcutPlatformUnsupported":
      "Atalhos de ditado não são suportados nesta plataforma.",
    "hud.error.shortcutDuplicate":
      "Os atalhos de segurar para falar e de ligar o ditado precisam ser diferentes.",
    "hud.error.recordingUnsupported": "A gravação de ditado não é suportada nesta plataforma.",
    "hud.error.helperRestarting": "O processo auxiliar de ditado está reiniciando.",
    "hud.error.composerUnavailable":
      "O ditado não conseguiu entregar o texto a este campo. Tente de novo.",
    "hud.meeting.detected": "Reunião detectada",
    "hud.meeting.record": "Gravar",
    "hud.meeting.dismiss": "Dispensar aviso de reunião",
    "hud.meeting.micInUse": "Microfone em uso",
    "hud.recording.windowTitle": "Gravação do Clovy",
    "hud.recording.recording": "Gravando. Clique para abrir o Clovy",
    "hud.recording.paused": "Pausado. Clique para abrir o Clovy",
    "hud.meetingEnd.title": "Reunião encerrada",
    "hud.meetingEnd.stop": "Parar agora",
    "hud.meetingEnd.stopLabel": "Parar a gravação agora",
    "hud.meetingEnd.keep": "Continuar gravando",
    "hud.meetingEnd.seconds": "{seconds}s",
    "hud.agents.windowTitle": "Agentes do Clovy",
    "hud.agents.activity": "Atividade dos agentes",
    "hud.agents.sessions": "Sessões de agentes",
    "hud.agents.hide": "Ocultar HUD de sessões",
    "hud.agents.expand": "Expandir atividade dos agentes",
    "hud.agents.collapse": "Recolher atividade dos agentes",
    "hud.agents.needInput": {
      one: "{count} precisa de resposta",
      other: "{count} precisam de resposta",
    },
    "hud.agents.running": { one: "{count} em execução", other: "{count}" },
    "hud.agents.activeAgents": { one: "{count} agente ativo", other: "{count} agentes ativos" },
    "hud.agents.status.thinking": "Pensando",
    "hud.agents.status.needsInput": "Precisa de resposta",
    "hud.agents.status.done": "Concluído",
    "hud.agents.status.failed": "Encontrou um problema",
    "hud.agents.status.stopped": "Parado",
    "hud.agents.untitledSession": "Sessão do agente",
  },
});

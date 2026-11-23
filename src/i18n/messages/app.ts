import { defineMessages } from "../define";

export default defineMessages({
  en: {
    // Tab titles and breadcrumb roots
    "app.nav.projects": "Projects",
    "app.nav.allNotes": "All notes",
    "app.nav.routines": "Routines",
    "app.nav.dictation": "Dictation",
    "app.nav.notes": "Notes",
    "app.nav.meetingNotes": "Meeting notes",
    "app.nav.today": "Today",
    "app.newNote": "New note",
    "app.breadcrumb.backTo": "Back to {name}",
    "app.breadcrumb.backToMeetingNotes": "Back to meeting notes",

    // Recording
    "app.recording.startExpired":
      "Recording did not start in time. Open meeting notes and select Record to try again.",
    "app.recording.micNotReady": "Microphone is not ready.",
    "app.recording.didNotStart": "Recording did not start.",
    "app.recording.stopBeforeDeletingNote": "Stop the current recording before deleting a note.",
    "app.recording.stopBeforeDeletingMeetings":
      "Stop the current recording before deleting meetings.",
    "app.inactivity.title": "Still in a meeting?",
    "app.inactivity.description": "Clovy has not heard meeting audio for a while.",
    "app.inactivity.pause": "Pause recording",
    "app.inactivity.keep": "Keep recording",
    "app.inactivity.countdown": {
      one: "Clovy will pause this recording in {count} seconds if you do not answer.",
      other: "Clovy will pause this recording in {count} seconds if you do not answer.",
    },

    // Updates
    "app.update.available": "Clovy {version} is available",
    "app.update.offerAria": "Clovy {version} is available",
    "app.update.install": "Update",
    "app.update.later": "Later",
    "app.update.waitingForCapture": "Clovy updates when the recording ends.",
    "app.update.restartAfterCapture": "Clovy restarts when the recording ends.",
    "app.update.upToDate": "Clovy is up to date.",
    "app.update.unknownError": "An unknown error occurred.",
    "app.update.checking": "Checking for updates...",
    "app.update.downloading": "Downloading update...",
    "app.update.preparing": "Installing update...",
    "app.update.failed": "Update failed: {message}",
    "app.update.checkFailed": "Update check failed: {message}",
    "app.update.relaunchFailed": "Restart failed: {message}",
    "app.update.relaunchCancelled":
      "Pending note edits could not be saved, so Clovy did not restart. Try again.",
    "app.update.captureActive": "A recording or dictation is running. Restart when it ends.",
    "app.update.relaunchAria": "Restart to update to Clovy {version}",
    "app.update.relaunching": "Restarting...",
    "app.update.relaunch": "Restart to update",
    "app.update.hideProgress": "Hide update progress",
    "app.update.dismissStatus": "Dismiss update status",

    // Shell chrome
    "app.sidebar.show": "Show sidebar",
    "app.sidebar.hide": "Hide sidebar",
    "app.sidebar.resize": "Resize sidebar",
    "app.sidebar.toggle": "Toggle sidebar",
    "app.workspace.loading": "Loading view",
    "app.workspace.loadFailedTitle": "Couldn't open this view",
    "app.workspace.loadFailedBody": "Clovy couldn't load this part of the app. Try again.",
    "app.workspace.openingNote": "Opening note",
    "app.workspace.couldNotOpen": "Could not open {title}.",

    // Notes, sessions, projects
    "app.deleteNote.title": "Delete note?",
    "app.deleteNote.description":
      "This permanently deletes the note and its transcript. This can't be undone.",
    "app.deleteNote.confirm": "Delete note",
    "app.audio.downloaded": "Audio downloaded",
    "app.audio.showFile": "Show file",
    "app.session.renameFailed": "Could not save the session name. It may revert after a restart.",
    "app.projects.added": { one: "{count} project added", other: "{count} projects added" },
  },
  "pt-BR": {
    "app.nav.projects": "Projetos",
    "app.nav.allNotes": "Todas as notas",
    "app.nav.routines": "Rotinas",
    "app.nav.dictation": "Ditado",
    "app.nav.notes": "Notas",
    "app.nav.meetingNotes": "Notas de reunião",
    "app.nav.today": "Hoje",
    "app.newNote": "Nova nota",
    "app.breadcrumb.backTo": "Voltar para {name}",
    "app.breadcrumb.backToMeetingNotes": "Voltar para notas de reunião",

    "app.recording.startExpired":
      "A gravação não começou a tempo. Abra as notas de reunião e selecione Gravar para tentar de novo.",
    "app.recording.micNotReady": "O microfone não está pronto.",
    "app.recording.didNotStart": "A gravação não começou.",
    "app.recording.stopBeforeDeletingNote": "Pare a gravação atual antes de excluir uma nota.",
    "app.recording.stopBeforeDeletingMeetings": "Pare a gravação atual antes de excluir reuniões.",
    "app.inactivity.title": "Ainda está em uma reunião?",
    "app.inactivity.description": "O Clovy não ouve o áudio da reunião há algum tempo.",
    "app.inactivity.pause": "Pausar gravação",
    "app.inactivity.keep": "Continuar gravando",
    "app.inactivity.countdown": {
      one: "O Clovy vai pausar esta gravação em {count} segundo se você não responder.",
      other: "O Clovy vai pausar esta gravação em {count} segundos se você não responder.",
    },

    "app.update.available": "Clovy {version} disponível",
    "app.update.offerAria": "Clovy {version} disponível",
    "app.update.install": "Atualizar",
    "app.update.later": "Depois",
    "app.update.waitingForCapture": "O Clovy atualiza quando a gravação terminar.",
    "app.update.restartAfterCapture": "O Clovy reinicia quando a gravação terminar.",
    "app.update.upToDate": "O Clovy está atualizado.",
    "app.update.unknownError": "Ocorreu um erro desconhecido.",
    "app.update.checking": "Procurando atualizações...",
    "app.update.downloading": "Baixando atualização...",
    "app.update.preparing": "Instalando atualização...",
    "app.update.failed": "Falha na atualização: {message}",
    "app.update.checkFailed": "Falha ao procurar atualizações: {message}",
    "app.update.relaunchFailed": "Falha ao reiniciar: {message}",
    "app.update.relaunchCancelled":
      "Não deu para salvar as edições pendentes, então o Clovy não reiniciou. Tente de novo.",
    "app.update.captureActive":
      "Uma gravação ou um ditado está em andamento. Reinicie quando terminar.",
    "app.update.relaunchAria": "Reiniciar para atualizar para o Clovy {version}",
    "app.update.relaunching": "Reiniciando...",
    "app.update.relaunch": "Reiniciar para atualizar",
    "app.update.hideProgress": "Ocultar progresso da atualização",
    "app.update.dismissStatus": "Dispensar status da atualização",

    "app.sidebar.show": "Mostrar barra lateral",
    "app.sidebar.hide": "Ocultar barra lateral",
    "app.sidebar.resize": "Redimensionar barra lateral",
    "app.sidebar.toggle": "Alternar barra lateral",
    "app.workspace.loading": "Carregando visualização",
    "app.workspace.loadFailedTitle": "Não foi possível abrir esta visualização",
    "app.workspace.loadFailedBody":
      "O Clovy não conseguiu carregar esta parte do app. Tente de novo.",
    "app.workspace.openingNote": "Abrindo nota",
    "app.workspace.couldNotOpen": "Não foi possível abrir {title}.",

    "app.deleteNote.title": "Excluir nota?",
    "app.deleteNote.description":
      "Isso exclui permanentemente a nota e a transcrição. Não é possível desfazer.",
    "app.deleteNote.confirm": "Excluir nota",
    "app.audio.downloaded": "Áudio baixado",
    "app.audio.showFile": "Mostrar arquivo",
    "app.session.renameFailed":
      "Não foi possível salvar o nome da sessão. Ele pode voltar ao anterior depois de reiniciar.",
    "app.projects.added": {
      one: "{count} projeto adicionado",
      other: "{count} projetos adicionados",
    },
  },
});

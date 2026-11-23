import { defineMessages } from "../define";

export default defineMessages({
  en: {
    "settings.localTranscription.engine": "Transcription engine",
    "settings.localTranscription.groq": "Groq",
    "settings.localTranscription.groqDescription": "Cloud, with your own key",
    "settings.localTranscription.local": "Local (Parakeet Ultra)",
    "settings.localTranscription.localDescription": "On this device, 25 languages",
    "settings.localTranscription.install": "Install model (632 MB)",
    "settings.localTranscription.required": "Install the model to use it",
    "settings.localTranscription.cancel": "Cancel",
    "settings.localTranscription.ready": "Ready · {size}",
    "settings.localTranscription.remove": "Remove",
    "settings.localTranscription.failed": "Failed: {reason}",
    "settings.localTranscription.networkFailed":
      "Could not download {file}. Check your connection and try again.",
    "settings.localTranscription.retry": "Try again",
    "settings.localTranscription.removed": "The model was removed. Transcription switched to Groq.",
    "settings.localTranscription.loading": "Checking model…",
    "settings.localTranscription.progress": "Model download",

    "settings.appearance.title": "Appearance",
    "settings.appearance.blurb":
      "Choose the language, theme, background, accent color, text size, and date format Clovy uses.",
    "settings.interfaceLanguage.title": "Interface language",
    "settings.interfaceLanguage.description":
      "The language of Clovy's menus, buttons, and messages. Dictation and note transcription keep their own language setting.",
    "settings.interfaceLanguage.aria": "Interface language: {language}",
    "settings.theme.title": "Theme",
    "settings.theme.description": "Match the system or force light or dark mode.",
    "settings.theme.aria": "App theme",
    "settings.theme.system": "System",
    "settings.theme.light": "Light",
    "settings.theme.dark": "Dark",
    "settings.theme.systemAria": "Match system theme",
    "settings.theme.lightAria": "Use light theme",
    "settings.theme.darkAria": "Use dark theme",
    "settings.background.title": "Background",
    "settings.background.description": "The background in New session and Home.",
    "settings.background.drift": "Drift",
    "settings.background.none": "None",
    "settings.background.image": "Image",
    "settings.background.choose": "Choose image",
    "settings.background.remove": "Remove",
    "settings.background.missing": "Image unavailable. Choose another file.",
    "settings.background.error":
      "Could not use this image. Choose a readable PNG, JPEG or WebP up to 20 MB and 8192 pixels per side. Try reducing its resolution.",
    "settings.textSize.title": "Text size",
    "settings.textSize.description":
      "Make text across the app larger. Applies to labels and notes.",
    "settings.textSize.default": "Default",
    "settings.textSize.large": "Large",
    "settings.textSize.larger": "Larger",
    "settings.textSize.optionAria": "{size} text size",
    "settings.accent.title": "Accent",
    "settings.accent.description":
      "The brand color used across buttons, highlights, and the recorder.",
    "settings.accent.aria": "Accent color: {color}",
    "settings.dateFormat.title": "Date format",
    "settings.dateFormat.description": "Choose how calendar dates appear.",
    "settings.dateFormat.aria": "Date format: {format}",
    "settings.dateFormat.system": "System",
    "settings.dateFormat.monthFirst": "Jul 9",
    "settings.dateFormat.dayFirst": "9 Jul",

    "settings.tabs.general": "General",
    "settings.tabs.shortcuts": "Shortcuts",
    "settings.tabs.dictation": "Dictation",
    "settings.tabs.audio": "Audio",
    "settings.tabs.models": "Models",
    "settings.tabs.agent": "Agent",
    "settings.tabs.memory": "Memory",
    "settings.tabs.connectors": "Plugins",
    "settings.tabs.about": "About",
    "settings.page.description": "Manage audio, dictation, AI models, and agent capabilities.",
    "settings.page.navAria": "Settings sections",
    "settings.general.blurb": "Everyday Clovy preferences.",
    "settings.updates.title": "Updates",
    "settings.updates.description":
      "Clovy looks for a new version each time it opens and asks before installing it.",
    "settings.updates.installed": "Installed version",
    "settings.updates.check": "Check for updates",
    "settings.updates.checking": "Checking...",

    "settings.shortcuts.blurb":
      "Set the keyboard shortcuts that start dictation and control Clovy.",
    "settings.shortcuts.modifierRequiredWindows":
      "Add Ctrl, Alt, Shift, or Win, or pick a key that works alone: Ctrl, Alt, or Shift on either side, F13 to F24, Pause, Scroll Lock, or Insert.",
    "settings.shortcuts.noKeyWindows":
      "That key doesn't reach Windows: the keyboard handles it itself, as most keyboards do with Fn. Try Right Ctrl.",
    "settings.shortcuts.unsupportedKeyWindows":
      "Pick a letter, number, punctuation, arrow, or function key.",
    "settings.shortcuts.registerFailedWindows":
      "Windows could not register {shortcut}. Another app may already use it.",
    "settings.shortcuts.captureHintWindows":
      "Press a combination, or Ctrl, Alt, or Shift alone, on either side: hold it to dictate. Esc cancels.",
    "settings.shortcuts.captureHintWindowsToggle":
      "Press a combination, or Ctrl, Alt, or Shift alone, on either side: tap it twice to start or stop. Esc cancels.",
    "settings.shortcuts.modifierRequiredMac": "Shortcut must include Cmd, Ctrl, Opt, Shift, or Fn.",
    "settings.shortcuts.monitorUnavailable": "Global shortcut monitoring is unavailable.",
    "settings.shortcuts.unavailable": "Dictation shortcut is unavailable.",
    "settings.shortcuts.pressToRecord": "Press the shortcut to record it.",
    "settings.shortcuts.captureFailed": "Shortcut could not be captured.",
    "settings.shortcuts.captureEnded": "Shortcut capture ended.",
    "settings.shortcuts.noTarget": "Shortcut capture returned without an active target.",
    "settings.shortcuts.invalidData": "Shortcut capture returned invalid data.",
    "settings.shortcuts.set": "{shortcut} set to {label}.",
    "settings.shortcuts.pushToTalk": "Push to talk",
    "settings.shortcuts.pushToTalkDescription":
      "Hold this shortcut to dictate, then release to paste.",
    "settings.shortcuts.toggle": "Toggle dictation",
    "settings.shortcuts.toggleDescription": "Press this shortcut to start or stop dictation.",
    "settings.shortcuts.unavailableTitle": "Dictation shortcuts unavailable",
    "settings.shortcuts.unavailableDescription":
      "Global dictation shortcuts are not available on this device.",
    "settings.shortcuts.change": "Change",
    "settings.shortcuts.reset": "Reset",
    "settings.shortcuts.resetAria": "Reset {name} shortcut to default",

    "settings.helper.restarting": "Dictation stopped and is restarting.",
    "settings.helper.failed": "Settings helper failed.",
    "settings.helper.unavailableAria": "Dictation unavailable",
    "settings.helper.pausedEyebrow": "Dictation paused",

    "settings.dictation.blurb": "Choose the language, microphone, and behavior for dictation.",
    "settings.dictation.languageDescription":
      "Default language hint for note transcription and dictation.",
    "settings.dictation.languageAria": "Default transcription language",
    "settings.dictation.languageSet": "Default transcription language set to {language}.",
    "settings.dictation.languageAuto": "Default transcription language set to auto-detect.",

    "settings.audio.blurbWindows": "Control how Clovy captures microphone audio on this device.",
    "settings.audio.blurb": "Control how Clovy captures meeting and system audio.",
    "settings.audio.microphone": "Microphone",
    "settings.audio.autoDetect": "Auto-detect",
    "settings.audio.microphoneDescription": "Input device used for dictation.",
    "settings.audio.autoDetectUses": "Auto-detect uses {name}.",
    "settings.audio.autoDetectSystem": "Auto-detect uses the current system input.",
    "settings.audio.microphoneSet": "Microphone set to {name}.",
    "settings.audio.microphoneAuto": "Microphone set to auto-detect.",
    "settings.audio.systemAudio": "System audio",
    "settings.audio.systemAudioDescription":
      "Capture audio from other apps along with your microphone.",
    "settings.audio.systemAudioAria": "Capture system audio for notes",

    "settings.micTest.title": "Mic test",
    "settings.micTest.levelAria": "Microphone test level",
    "settings.micTest.startOver": "Start over",
    "settings.micTest.start": "Start test",
    "settings.micTest.recording": "Recording 5-second sample.",
    "settings.micTest.ready": "Sample ready. Check volume.",
    "settings.micTest.idle": "Check your microphone.",
    "settings.micTest.playbackAria": "Microphone test playback progress",
    "settings.micTest.playAria": "Play microphone test sample",
    "settings.micTest.play": "Play sample",
    "settings.micTest.noSample": "Microphone test did not return a playable sample.",
    "settings.micTest.couldNotRecord": "Microphone test could not record.",
    "settings.micTest.playbackUnavailable":
      "Microphone test recorded, but playback is unavailable.",

    "settings.models.blurb": "Choose the models Clovy uses for voice, text, image, and video.",
    "settings.models.voice": "Voice",
    "settings.models.voiceDescription":
      "Choose the model Clovy uses for note transcription and dictation.",
    "settings.models.partitionNote":
      "Showing models for the current data set: {name}. Switch to the default data set to edit global models.",
    "settings.models.transcription": "Transcription",
    "settings.models.transcriptionDescription": "Speech-to-text for note recordings and dictation.",
    "settings.models.moreVoiceAria": "More options for voice",
    "settings.models.moreVoiceDescription": "Advanced voice settings",
    "settings.models.text": "Text",
    "settings.models.textGroupDescription": "Choose an OpenRouter text model.",
    "settings.models.textDescription": "This model generates notes when Notes uses OpenRouter.",
    "settings.models.textOtherEngineDescription":
      "Notes are generated by {engine}. This text model is not used for notes.",
    "settings.models.catalogFailed":
      "The model list could not be loaded. Your saved model is still selected.",
    "settings.models.retryCatalog": "Retry model list",
    "settings.models.moreTextAria": "More options for text",
    "settings.models.moreTextDescription": "Advanced text settings",
    "settings.models.imageAndVideo": "Image and video",
    "settings.models.mediaDescription":
      "Choose the models Clovy uses when you ask it to generate an image or video.",
    "settings.models.image": "Image",
    "settings.models.imageDescription": "Used by the image generation tool.",
    "settings.models.video": "Video",
    "settings.models.videoDescription": "Used by the video generation tool.",
    "settings.models.moreMediaAria": "More options for image and video",
    "settings.models.moreMediaDescription": "Advanced image and video settings",
    "settings.models.updated.transcription": "Transcription model updated.",
    "settings.models.updated.image": "Image model updated.",
    "settings.models.updated.video": "Video model updated.",
    "settings.models.updated.generation": "Text model updated.",
    "settings.models.modelLabel.transcription": "transcription model",
    "settings.models.modelLabel.generation": "text model",
    "settings.models.modelLabel.image": "image model",
    "settings.models.modelLabel.video": "video model",
    "settings.models.popoverTitle.transcription": "Transcription model",
    "settings.models.popoverTitle.image": "Image model",
    "settings.models.popoverTitle.video": "Video model",
    "settings.models.changeAria": "Change {model}",
    "settings.models.chooseAria": "Choose {model}",

    "settings.liveTranscription.title": "Live transcription",
    "settings.liveTranscription.description":
      "Show a live transcript with the selected transcription engine. With Groq, this transcribes audio twice and may increase provider usage. Local processes both passes on this device. Turning it off shows the transcript after recording ends.",
    "settings.liveTranscription.aria": "Show a live transcript while recording",
    "settings.liveTranscription.on":
      "Live transcription on: the transcript streams while you record.",
    "settings.liveTranscription.off":
      "Live transcription off: the transcript appears after the recording ends.",

    "settings.safeMode.title": "Safe mode",
    "settings.safeMode.descriptionWithVideo":
      "Ask the image provider to filter adult content in generated and edited images, and hold back video prompts that request it. On by default. Turning it off lifts the filter only where the provider allows it.",
    "settings.safeMode.description":
      "Ask the image provider to filter adult content in generated and edited images. On by default. Turning it off lifts the filter only where the provider allows it.",
    "settings.safeMode.aria": "Filter adult content in images",
    "settings.safeMode.on": "Safe mode on: adult content is filtered.",
    "settings.safeMode.off":
      "Safe mode off: the filter is lifted only where the provider allows it.",

    "settings.providerKeys.title": "Provider API keys",
    "settings.providerKeys.description":
      "Your keys stay in the local credential store and are sent only to their own provider.",
    "settings.providerKeys.apiKey": "API key",
    "settings.providerKeys.configured": "Configured",
    "settings.providerKeys.missing": "Not configured",
    "settings.providerKeys.savedHidden": "Saved key hidden",
    "settings.providerKeys.enterKey": "Enter an API key before saving.",
    "settings.providerKeys.saved": "{provider} API key saved.",
    "settings.providerKeys.removed": "{provider} API key removed.",
    "settings.providerKeys.groq.name": "Groq",
    "settings.providerKeys.groq.description": "Transcription for dictation and notes",
    "settings.providerKeys.openrouter.name": "OpenRouter",
    "settings.providerKeys.openrouter.description": "Text models for notes and transcript cleanup.",
    "settings.providerKeys.fal.name": "fal",
    "settings.providerKeys.fal.description": "Images and video",
    "settings.providerKeys.tavily.name": "Tavily",
    "settings.providerKeys.tavily.description": "Web search",

    "settings.about.blurb": "Version, release channel, and other details about this copy of Clovy.",
    "settings.about.releaseVersion": "Release version",
    "settings.about.commit": "Commit",

    "settings.about.replayOnboarding": "Replay onboarding",
    "settings.about.replayOnboardingDescription":
      "Dev only. Forget that onboarding finished and reload into the first-run wizard.",

    "settings.experiments.unlocked": "Experiments are unlocked",
    "settings.experiments.title": "Experiments",
    "settings.experiments.description":
      "Runtime overrides for features that ship dark. They apply to this install only.",
    "settings.experiments.hide": "Hide again",
    "settings.experiments.browserUse": "Browser use",
    "settings.experiments.browserUseDescription":
      "Enable Browser use on this install while the public feature remains off. Turning it off applies fully after Clovy restarts.",
    "settings.experiments.browserUseAria": "Enable experimental Browser use",

    "settings.startup.hideDock": "Hide from Dock",
    "settings.startup.hideDockDescription": "Hide the Dock icon when the main window is closed.",
    "settings.startup.dockReadError": "Could not read the Dock preference.",
    "settings.startup.dockUpdateError": "Could not update the Dock preference. Try again.",
    "settings.startup.readError": "Could not read the login item state.",
    "settings.startup.updateError": "Could not update the login item. Try again.",
    "settings.startup.title": "Startup",
    "settings.startup.description":
      "Dictation shortcuts and meeting detection only work while Clovy is running.",
    "settings.startup.openAtLogin": "Open Clovy at login",
    "settings.startup.openAtLoginDescription":
      "Start Clovy automatically when you sign in to your computer.",

    "settings.permissions.systemTitle": "System permissions",
    "settings.permissions.audioAccessTitle": "Audio access",
    "settings.permissions.macDescription":
      "macOS access used for recording audio, pasting dictation, and capturing system sound.",
    "settings.permissions.systemAudioDescription":
      "Audio sources available for recording microphone and app audio.",
    "settings.permissions.micOnlyDescription":
      "Audio sources available for recording microphone audio.",
    "settings.permissions.microphoneDescription": "Record dictation and note audio.",
    "settings.permissions.accessibility": "Accessibility",
    "settings.permissions.accessibilityDescription": "Paste dictated text into the active app.",
    "settings.permissions.systemAudioRowDescription":
      "Record audio from other apps when system audio is enabled.",
    "settings.permissions.manage": "Manage",
    "settings.permissions.manageAria": "Manage {name} permission",
    "settings.permissions.status.allowed": "Allowed",
    "settings.permissions.status.blocked": "Blocked",
    "settings.permissions.status.restricted": "Restricted",
    "settings.permissions.status.needsAccess": "Needs access",
    "settings.permissions.status.notRequested": "Not requested",
    "settings.permissions.status.noMicrophone": "No microphone found",
    "settings.permissions.status.unsupported": "Unsupported",
    "settings.permissions.status.unknown": "Unknown",
    "settings.permissions.status.checking": "Checking",
    "settings.permissions.status.available": "Available",
    "settings.permissions.status.unavailable": "Unavailable",

    "settings.style.title": "Style",
    "settings.style.hudPlacement": "Indicator position",
    "settings.style.hudPlacementDescription": "Where the dictation pill appears on screen.",
    "settings.style.hudPlacementBottom": "Bottom",
    "settings.style.hudPlacementTop": "Top",
    "settings.style.cleanup": "Clean up dictation text",
    "settings.style.cleanupDescription":
      "Fixes punctuation and filler words with an OpenRouter model before pasting. Off pastes the transcript as is, which is faster.",
    "settings.style.cleanupModel": "Cleanup model",
    "settings.style.cleanupModelDescription":
      "Runs as one request per dictation. Uses your OpenRouter key when you saved one.",
    "settings.style.outputStyle": "Output style",
    "settings.style.aria": "Dictation style",
    "settings.style.standard": "Standard",
    "settings.style.casual": "Casual",
    "settings.style.formal": "Formal",
    "settings.style.standardDescription":
      "Sentence case with light cleanup. Keeps your natural tone.",
    "settings.style.casualDescription": "Lowercase sentences, contractions, minimal cleanup.",
    "settings.style.formalDescription":
      "Full words and conventional capitalization. Keeps your wording.",
    "settings.style.standardSample":
      "Got it. Let me know when you're free to chat about the Q3 plan. Happy to jump on in the morning.",
    "settings.style.casualSample":
      "got it. let me know when you're free to chat about the q3 plan. happy to jump on in the morning.",
    "settings.style.formalSample":
      "Got it. Let me know when you are free to chat about the Q3 plan. Happy to jump on in the morning.",

    "settings.dictionary.title": "Dictionary",
    "settings.dictionary.description":
      "Words or phrases Clovy should preserve during transcription.",
    "settings.dictionary.empty":
      "No entries yet. Add words or phrases for transcription to preserve.",
    "settings.dictionary.noMatch": 'No entries match "{query}".',
    "settings.dictionary.searchAria": "Search dictionary",
    "settings.dictionary.add": "Add entry",
    "settings.dictionary.editAria": "Edit {phrase}",
    "settings.dictionary.deleteAria": "Delete {phrase}",
    "settings.dictionary.editTitle": "Edit dictionary entry",
    "settings.dictionary.addTitle": "Add dictionary entry",
    "settings.dictionary.saveChanges": "Save changes",
    "settings.dictionary.field": "Word or phrase",
    "settings.dictionary.placeholder": "e.g. Anthropic, ARR, Jane Doe",

    "settings.personality.loadError": "Unable to load Clovy's personality.",
    "settings.personality.saveError": "Unable to save Clovy's personality.",
    "settings.personality.saved": "Saved",
    "settings.personality.title": "Personality",
    "settings.personality.description":
      "Choose the voice Clovy uses in Home and new agent sessions.",
    "settings.personality.loading": "Loading Clovy's personality",
    "settings.personality.legend": "Choose Clovy's personality",
    "settings.personality.applies": "Applies to future Home replies and the next agent run.",
  },
  "pt-BR": {
    "settings.localTranscription.engine": "Motor de transcrição",
    "settings.localTranscription.groq": "Groq",
    "settings.localTranscription.groqDescription": "Na nuvem, com sua chave",
    "settings.localTranscription.local": "Local (Parakeet Ultra)",
    "settings.localTranscription.localDescription": "No dispositivo, 25 idiomas",
    "settings.localTranscription.install": "Instalar modelo (632 MB)",
    "settings.localTranscription.required": "Instale o modelo para usar",
    "settings.localTranscription.cancel": "Cancelar",
    "settings.localTranscription.ready": "Pronto · {size}",
    "settings.localTranscription.remove": "Remover",
    "settings.localTranscription.failed": "Falhou: {reason}",
    "settings.localTranscription.networkFailed":
      "Não foi possível baixar {file}. Verifique sua conexão e tente de novo.",
    "settings.localTranscription.retry": "Tentar de novo",
    "settings.localTranscription.removed": "O modelo foi removido. A transcrição mudou para Groq.",
    "settings.localTranscription.loading": "Verificando modelo…",
    "settings.localTranscription.progress": "Download do modelo",

    "settings.appearance.title": "Aparência",
    "settings.appearance.blurb":
      "Escolha o idioma, o tema, o fundo, a cor de destaque, o tamanho do texto e o formato de data do Clovy.",
    "settings.interfaceLanguage.title": "Idioma da interface",
    "settings.interfaceLanguage.description":
      "O idioma dos menus, botões e mensagens do Clovy. Ditado e transcrição de notas mantêm a própria configuração de idioma.",
    "settings.interfaceLanguage.aria": "Idioma da interface: {language}",
    "settings.theme.title": "Tema",
    "settings.theme.description": "Siga o sistema ou force o modo claro ou escuro.",
    "settings.theme.aria": "Tema do app",
    "settings.theme.system": "Sistema",
    "settings.theme.light": "Claro",
    "settings.theme.dark": "Escuro",
    "settings.theme.systemAria": "Seguir o tema do sistema",
    "settings.theme.lightAria": "Usar tema claro",
    "settings.theme.darkAria": "Usar tema escuro",
    "settings.background.title": "Fundo",
    "settings.background.description": "O fundo da Nova sessão e do Início.",
    "settings.background.drift": "Drift",
    "settings.background.none": "Nenhum",
    "settings.background.image": "Imagem",
    "settings.background.choose": "Escolher imagem",
    "settings.background.remove": "Remover",
    "settings.background.missing": "Imagem indisponível. Escolha outro arquivo.",
    "settings.background.error":
      "Não foi possível usar esta imagem. Escolha um PNG, JPEG ou WebP legível de até 20 MB e 8192 pixels por lado. Tente reduzir a resolução.",
    "settings.textSize.title": "Tamanho do texto",
    "settings.textSize.description":
      "Aumente o texto em todo o app. Afeta todos os rótulos, notas e conversas.",
    "settings.textSize.default": "Padrão",
    "settings.textSize.large": "Grande",
    "settings.textSize.larger": "Maior",
    "settings.textSize.optionAria": "Tamanho de texto {size}",
    "settings.accent.title": "Destaque",
    "settings.accent.description": "A cor da marca usada em botões, realces e no gravador.",
    "settings.accent.aria": "Cor de destaque: {color}",
    "settings.dateFormat.title": "Formato de data",
    "settings.dateFormat.description":
      "Escolha como as datas de sessões mais antigas aparecem na barra lateral.",
    "settings.dateFormat.aria": "Formato de data: {format}",
    "settings.dateFormat.system": "Sistema",
    "settings.dateFormat.monthFirst": "jul 9",
    "settings.dateFormat.dayFirst": "9 jul",

    "settings.tabs.general": "Geral",
    "settings.tabs.shortcuts": "Atalhos",
    "settings.tabs.dictation": "Ditado",
    "settings.tabs.audio": "Áudio",
    "settings.tabs.models": "Modelos",
    "settings.tabs.agent": "Agente",
    "settings.tabs.memory": "Memória",
    "settings.tabs.connectors": "Plugins",
    "settings.tabs.about": "Sobre",
    "settings.page.description": "Gerencie áudio, ditado, modelos de IA e recursos do agente.",
    "settings.page.navAria": "Seções das configurações",
    "settings.general.blurb": "As preferências do dia a dia do Clovy.",
    "settings.updates.title": "Atualizações",
    "settings.updates.description":
      "O Clovy procura uma versão nova sempre que abre e pergunta antes de instalar.",
    "settings.updates.installed": "Versão instalada",
    "settings.updates.check": "Procurar atualizações",
    "settings.updates.checking": "Procurando...",

    "settings.shortcuts.blurb":
      "Defina os atalhos de teclado que iniciam o ditado e controlam o Clovy.",
    "settings.shortcuts.modifierRequiredWindows":
      "Inclua Ctrl, Alt, Shift ou Win, ou escolha uma tecla que funciona sozinha: Ctrl, Alt ou Shift de qualquer lado, F13 a F24, Pause, Scroll Lock ou Insert.",
    "settings.shortcuts.noKeyWindows":
      "Essa tecla não chega ao Windows: o próprio teclado a trata, como a maioria faz com o Fn. Tente o Ctrl direito.",
    "settings.shortcuts.unsupportedKeyWindows":
      "Escolha uma letra, número, pontuação, seta ou tecla de função.",
    "settings.shortcuts.registerFailedWindows":
      "O Windows não conseguiu registrar {shortcut}. Outro app pode já estar usando.",
    "settings.shortcuts.captureHintWindows":
      "Pressione uma combinação, ou Ctrl, Alt ou Shift sozinho, de qualquer lado: segure para ditar. Esc cancela.",
    "settings.shortcuts.captureHintWindowsToggle":
      "Pressione uma combinação, ou Ctrl, Alt ou Shift sozinho, de qualquer lado: dois toques ligam e desligam. Esc cancela.",
    "settings.shortcuts.modifierRequiredMac":
      "O atalho precisa incluir Cmd, Ctrl, Opt, Shift ou Fn.",
    "settings.shortcuts.monitorUnavailable":
      "O monitoramento de atalhos globais não está disponível.",
    "settings.shortcuts.unavailable": "O atalho de ditado não está disponível.",
    "settings.shortcuts.pressToRecord": "Pressione o atalho para registrá-lo.",
    "settings.shortcuts.captureFailed": "Não foi possível capturar o atalho.",
    "settings.shortcuts.captureEnded": "Captura de atalho encerrada.",
    "settings.shortcuts.noTarget": "A captura de atalho retornou sem um destino ativo.",
    "settings.shortcuts.invalidData": "A captura de atalho retornou dados inválidos.",
    "settings.shortcuts.set": "{shortcut} definido como {label}.",
    "settings.shortcuts.pushToTalk": "Pressione para falar",
    "settings.shortcuts.pushToTalkDescription": "Segure este atalho para ditar e solte para colar.",
    "settings.shortcuts.toggle": "Alternar ditado",
    "settings.shortcuts.toggleDescription": "Pressione este atalho para iniciar ou parar o ditado.",
    "settings.shortcuts.unavailableTitle": "Atalhos de ditado indisponíveis",
    "settings.shortcuts.unavailableDescription":
      "Os atalhos globais de ditado não estão disponíveis neste dispositivo.",
    "settings.shortcuts.change": "Alterar",
    "settings.shortcuts.reset": "Redefinir",
    "settings.shortcuts.resetAria": "Redefinir o atalho {name} para o padrão",

    "settings.helper.restarting": "O ditado parou e está reiniciando.",
    "settings.helper.failed": "O auxiliar das configurações falhou.",
    "settings.helper.unavailableAria": "Ditado indisponível",
    "settings.helper.pausedEyebrow": "Ditado pausado",

    "settings.dictation.blurb": "Escolha o idioma, o microfone e o comportamento do ditado.",
    "settings.dictation.languageDescription":
      "Idioma padrão sugerido para a transcrição de notas e o ditado.",
    "settings.dictation.languageAria": "Idioma padrão de transcrição",
    "settings.dictation.languageSet": "Idioma padrão de transcrição definido como {language}.",
    "settings.dictation.languageAuto":
      "Idioma padrão de transcrição definido para detecção automática.",

    "settings.audio.blurbWindows":
      "Controle como o Clovy capta o áudio do microfone neste dispositivo.",
    "settings.audio.blurb": "Controle como o Clovy capta o áudio de reuniões e do sistema.",
    "settings.audio.microphone": "Microfone",
    "settings.audio.autoDetect": "Detecção automática",
    "settings.audio.microphoneDescription": "Dispositivo de entrada usado para o ditado.",
    "settings.audio.autoDetectUses": "A detecção automática usa {name}.",
    "settings.audio.autoDetectSystem": "A detecção automática usa a entrada atual do sistema.",
    "settings.audio.microphoneSet": "Microfone definido como {name}.",
    "settings.audio.microphoneAuto": "Microfone definido para detecção automática.",
    "settings.audio.systemAudio": "Áudio do sistema",
    "settings.audio.systemAudioDescription":
      "Capte o áudio de outros apps junto com o seu microfone.",
    "settings.audio.systemAudioAria": "Capturar o áudio do sistema para as notas",

    "settings.micTest.title": "Teste do microfone",
    "settings.micTest.levelAria": "Nível do teste do microfone",
    "settings.micTest.startOver": "Recomeçar",
    "settings.micTest.start": "Iniciar teste",
    "settings.micTest.recording": "Gravando uma amostra de 5 segundos.",
    "settings.micTest.ready": "Amostra pronta. Confira o volume.",
    "settings.micTest.idle": "Teste o seu microfone.",
    "settings.micTest.playbackAria": "Progresso da reprodução do teste do microfone",
    "settings.micTest.playAria": "Reproduzir a amostra do teste do microfone",
    "settings.micTest.play": "Reproduzir amostra",
    "settings.micTest.noSample": "O teste do microfone não retornou uma amostra reproduzível.",
    "settings.micTest.couldNotRecord": "Não foi possível gravar o teste do microfone.",
    "settings.micTest.playbackUnavailable":
      "O teste do microfone foi gravado, mas a reprodução não está disponível.",

    "settings.models.blurb": "Escolha os modelos que o Clovy usa para voz, texto, imagem e vídeo.",
    "settings.models.voice": "Voz",
    "settings.models.voiceDescription":
      "Escolha o modelo que o Clovy usa para a transcrição de notas e o ditado.",
    "settings.models.partitionNote":
      "Mostrando os modelos do conjunto de dados atual: {name}. Mude para o conjunto de dados padrão para editar os modelos globais.",
    "settings.models.transcription": "Transcrição",
    "settings.models.transcriptionDescription":
      "Conversão de fala em texto para gravações de notas e ditado.",
    "settings.models.moreVoiceAria": "Mais opções de voz",
    "settings.models.moreVoiceDescription": "Configurações avançadas de voz",
    "settings.models.text": "Texto",
    "settings.models.textGroupDescription": "Escolha um modelo de texto da OpenRouter.",
    "settings.models.textDescription":
      "Este modelo gera as notas quando o motor de Notas é OpenRouter.",
    "settings.models.textOtherEngineDescription":
      "As notas são geradas por {engine}. Este modelo de texto não é usado nas notas.",
    "settings.models.catalogFailed":
      "Não foi possível carregar a lista de modelos. Seu modelo salvo continua selecionado.",
    "settings.models.retryCatalog": "Tentar carregar a lista novamente",
    "settings.models.moreTextAria": "Mais opções de texto",
    "settings.models.moreTextDescription": "Configurações avançadas de texto",
    "settings.models.imageAndVideo": "Imagem e vídeo",
    "settings.models.mediaDescription":
      "Escolha os modelos que o Clovy usa quando você pede para gerar uma imagem ou um vídeo.",
    "settings.models.image": "Imagem",
    "settings.models.imageDescription": "Usado quando você gera uma imagem pela conversa.",
    "settings.models.video": "Vídeo",
    "settings.models.videoDescription": "Usado quando você gera um vídeo pela conversa.",
    "settings.models.moreMediaAria": "Mais opções de imagem e vídeo",
    "settings.models.moreMediaDescription": "Configurações avançadas de imagem e vídeo",
    "settings.models.updated.transcription": "Modelo de transcrição atualizado.",
    "settings.models.updated.image": "Modelo de imagem atualizado.",
    "settings.models.updated.video": "Modelo de vídeo atualizado.",
    "settings.models.updated.generation": "Modelo de texto atualizado.",
    "settings.models.modelLabel.transcription": "modelo de transcrição",
    "settings.models.modelLabel.generation": "modelo de texto",
    "settings.models.modelLabel.image": "modelo de imagem",
    "settings.models.modelLabel.video": "modelo de vídeo",
    "settings.models.popoverTitle.transcription": "Modelo de transcrição",
    "settings.models.popoverTitle.image": "Modelo de imagem",
    "settings.models.popoverTitle.video": "Modelo de vídeo",
    "settings.models.changeAria": "Alterar {model}",
    "settings.models.chooseAria": "Escolher {model}",

    "settings.liveTranscription.title": "Transcrição ao vivo",
    "settings.liveTranscription.description":
      "Mostra a transcrição ao vivo com o motor escolhido. Com Groq, isso transcreve o áudio duas vezes e pode aumentar o uso do provedor. Com Local, as duas transcrições acontecem no dispositivo. Se desativar, a transcrição aparece depois que a gravação termina.",
    "settings.liveTranscription.aria": "Mostrar uma transcrição ao vivo durante a gravação",
    "settings.liveTranscription.on":
      "Transcrição ao vivo ativada: a transcrição aparece enquanto você grava.",
    "settings.liveTranscription.off":
      "Transcrição ao vivo desativada: a transcrição aparece depois que a gravação termina.",

    "settings.safeMode.title": "Modo seguro",
    "settings.safeMode.descriptionWithVideo":
      "Pede ao provedor de imagens que filtre conteúdo adulto em imagens geradas e editadas e retém prompts de vídeo que peçam esse conteúdo. Ativado por padrão. Ao desativar, o filtro só é removido onde o provedor permite.",
    "settings.safeMode.description":
      "Pede ao provedor de imagens que filtre conteúdo adulto em imagens geradas e editadas. Ativado por padrão. Ao desativar, o filtro só é removido onde o provedor permite.",
    "settings.safeMode.aria": "Filtrar conteúdo adulto em imagens",
    "settings.safeMode.on": "Modo seguro ativado: conteúdo adulto é filtrado.",
    "settings.safeMode.off":
      "Modo seguro desativado: o filtro só é removido onde o provedor permite.",

    "settings.providerKeys.title": "Chaves de API de provedores",
    "settings.providerKeys.description":
      "Suas chaves ficam no armazenamento local de credenciais e são enviadas apenas ao respectivo provedor.",
    "settings.providerKeys.apiKey": "Chave de API",
    "settings.providerKeys.configured": "Configurada",
    "settings.providerKeys.missing": "Não configurada",
    "settings.providerKeys.savedHidden": "Chave salva oculta",
    "settings.providerKeys.enterKey": "Informe uma chave de API antes de salvar.",
    "settings.providerKeys.saved": "Chave de API de {provider} salva.",
    "settings.providerKeys.removed": "Chave de API de {provider} removida.",
    "settings.providerKeys.groq.name": "Groq",
    "settings.providerKeys.groq.description": "Transcrição para ditado e notas",
    "settings.providerKeys.openrouter.name": "OpenRouter",
    "settings.providerKeys.openrouter.description":
      "Modelos de texto para notas e limpeza de transcrições.",
    "settings.providerKeys.fal.name": "fal",
    "settings.providerKeys.fal.description": "Imagens e vídeo",
    "settings.providerKeys.tavily.name": "Tavily",
    "settings.providerKeys.tavily.description": "Busca na web",

    "settings.about.blurb": "Versão, canal de lançamento e outros detalhes desta cópia do Clovy.",
    "settings.about.releaseVersion": "Versão",
    "settings.about.commit": "Commit",

    "settings.about.replayOnboarding": "Repetir a introdução",
    "settings.about.replayOnboardingDescription":
      "Só em desenvolvimento. Esquece que a introdução foi concluída e recarrega no assistente inicial.",

    "settings.experiments.unlocked": "Experimentos desbloqueados",
    "settings.experiments.title": "Experimentos",
    "settings.experiments.description":
      "Substituições em tempo de execução para recursos lançados ocultos. Valem só para esta instalação.",
    "settings.experiments.hide": "Ocultar de novo",
    "settings.experiments.browserUse": "Uso do navegador",
    "settings.experiments.browserUseDescription":
      "Ativa o uso do navegador nesta instalação enquanto o recurso público continua desativado. Desativar só tem efeito completo depois que o Clovy reiniciar.",
    "settings.experiments.browserUseAria": "Ativar o uso do navegador experimental",

    "settings.startup.hideDock": "Ocultar do Dock",
    "settings.startup.hideDockDescription":
      "Ocultar o ícone do Dock quando a janela principal estiver fechada.",
    "settings.startup.dockReadError": "Não foi possível ler a preferência do Dock.",
    "settings.startup.dockUpdateError":
      "Não foi possível atualizar a preferência do Dock. Tente de novo.",
    "settings.startup.readError": "Não foi possível ler o estado do item de login.",
    "settings.startup.updateError": "Não foi possível atualizar o item de login. Tente de novo.",
    "settings.startup.title": "Inicialização",
    "settings.startup.description":
      "Os atalhos de ditado e a detecção de reuniões só funcionam enquanto o Clovy está aberto.",
    "settings.startup.openAtLogin": "Abrir o Clovy ao iniciar a sessão",
    "settings.startup.openAtLoginDescription":
      "Inicia o Clovy automaticamente quando você entra no computador.",

    "settings.permissions.systemTitle": "Permissões do sistema",
    "settings.permissions.audioAccessTitle": "Acesso ao áudio",
    "settings.permissions.macDescription":
      "Acessos do macOS usados para gravar áudio, colar o ditado e captar o som do sistema.",
    "settings.permissions.systemAudioDescription":
      "Fontes de áudio disponíveis para gravar o microfone e o áudio de apps.",
    "settings.permissions.micOnlyDescription":
      "Fontes de áudio disponíveis para gravar o áudio do microfone.",
    "settings.permissions.microphoneDescription": "Grava o áudio do ditado e das notas.",
    "settings.permissions.accessibility": "Acessibilidade",
    "settings.permissions.accessibilityDescription": "Cola o texto ditado no app ativo.",
    "settings.permissions.systemAudioRowDescription":
      "Grava o áudio de outros apps quando o áudio do sistema está ativado.",
    "settings.permissions.manage": "Gerenciar",
    "settings.permissions.manageAria": "Gerenciar a permissão de {name}",
    "settings.permissions.status.allowed": "Permitido",
    "settings.permissions.status.blocked": "Bloqueado",
    "settings.permissions.status.restricted": "Restrito",
    "settings.permissions.status.needsAccess": "Precisa de acesso",
    "settings.permissions.status.notRequested": "Não solicitado",
    "settings.permissions.status.noMicrophone": "Nenhum microfone encontrado",
    "settings.permissions.status.unsupported": "Sem suporte",
    "settings.permissions.status.unknown": "Desconhecido",
    "settings.permissions.status.checking": "Verificando",
    "settings.permissions.status.available": "Disponível",
    "settings.permissions.status.unavailable": "Indisponível",

    "settings.style.title": "Estilo",
    "settings.style.hudPlacement": "Posição do indicador",
    "settings.style.hudPlacementDescription": "Onde a pílula do ditado aparece na tela.",
    "settings.style.hudPlacementBottom": "Embaixo",
    "settings.style.hudPlacementTop": "Em cima",
    "settings.style.cleanup": "Limpar texto do ditado",
    "settings.style.cleanupDescription":
      "Corrige pontuação e vícios de fala com um modelo da OpenRouter antes de colar. Desligado, cola o transcrito como veio, mais rápido.",
    "settings.style.cleanupModel": "Modelo da limpeza",
    "settings.style.cleanupModelDescription":
      "Roda numa requisição por ditado. Usa a sua chave da OpenRouter quando ela está salva.",
    "settings.style.outputStyle": "Estilo de saída",
    "settings.style.aria": "Estilo do ditado",
    "settings.style.standard": "Padrão",
    "settings.style.casual": "Casual",
    "settings.style.formal": "Formal",
    "settings.style.standardDescription":
      "Maiúscula no início das frases, com uma limpeza leve. Mantém o seu tom natural.",
    "settings.style.casualDescription": "Frases em minúsculas, contrações e limpeza mínima.",
    "settings.style.formalDescription":
      "Palavras completas e maiúsculas convencionais. Mantém as suas palavras.",
    "settings.style.standardSample":
      "Entendi. Me avisa quando puder conversar sobre o plano do T3. Posso entrar de manhã, sem problema.",
    "settings.style.casualSample":
      "entendi. me avisa quando puder conversar sobre o plano do t3. posso entrar de manhã, sem problema.",
    "settings.style.formalSample":
      "Entendido. Avise-me quando estiver disponível para conversar sobre o plano do T3. Posso participar pela manhã, sem problema.",

    "settings.dictionary.title": "Dicionário",
    "settings.dictionary.description":
      "Palavras ou frases que o Clovy deve preservar na transcrição.",
    "settings.dictionary.empty":
      "Nenhuma entrada ainda. Adicione palavras ou frases para a transcrição preservar.",
    "settings.dictionary.noMatch": 'Nenhuma entrada corresponde a "{query}".',
    "settings.dictionary.searchAria": "Buscar no dicionário",
    "settings.dictionary.add": "Adicionar entrada",
    "settings.dictionary.editAria": "Editar {phrase}",
    "settings.dictionary.deleteAria": "Excluir {phrase}",
    "settings.dictionary.editTitle": "Editar entrada do dicionário",
    "settings.dictionary.addTitle": "Adicionar entrada ao dicionário",
    "settings.dictionary.saveChanges": "Salvar alterações",
    "settings.dictionary.field": "Palavra ou frase",
    "settings.dictionary.placeholder": "ex.: Anthropic, ARR, Maria Silva",

    "settings.personality.loadError": "Não foi possível carregar a personalidade do Clovy.",
    "settings.personality.saveError": "Não foi possível salvar a personalidade do Clovy.",
    "settings.personality.saved": "Salvo",
    "settings.personality.title": "Personalidade",
    "settings.personality.description":
      "Escolha a voz que o Clovy usa no Início e em novas sessões do agente.",
    "settings.personality.loading": "Carregando a personalidade do Clovy",
    "settings.personality.legend": "Escolha a personalidade do Clovy",
    "settings.personality.applies":
      "Vale para as próximas respostas no Início e a próxima execução do agente.",
  },
});

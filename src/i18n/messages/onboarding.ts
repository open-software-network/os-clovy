import { defineMessages } from "../define";

export default defineMessages({
  en: {
    // Flow chrome
    "onboarding.progress": "Setup progress: step {step} of {total}",
    "onboarding.skipForNow": "Skip for now",

    // Area
    "onboarding.area.title": "Where could I help most?",
    "onboarding.area.subtitle": "Pick the part of life you'd most like me to make easier.",
    "onboarding.area.legend": "Choose where Clovy should help first",
    "onboarding.area.work.label": "Work",
    "onboarding.area.work.description": "Meetings, follow-ups, and focused work",
    "onboarding.area.personal.label": "Personal",
    "onboarding.area.personal.description": "Plans, journaling, and everyday life",
    "onboarding.area.thinking.label": "Thinking",
    "onboarding.area.thinking.description": "Ideas, writing, and clearer decisions",
    "onboarding.area.play.label": "Play",
    "onboarding.area.play.description": "Stories, characters, and creative projects",

    // Mood / personality
    "onboarding.mood.title": "Choose my personality",
    "onboarding.mood.subtitle": "Pick the mood for our first conversation.",
    "onboarding.mood.legend": "Choose Clovy's greeting mood",
    "onboarding.mood.saveError": "I couldn't save this yet. Try again.",
    "onboarding.mood.status": "{mood} tone. {greeting}",
    "onboarding.mood.characterTitle": "{mood} Clovy",
    "onboarding.mood.calm.label": "Calm",
    "onboarding.mood.calm.description": "Steady, warm, and unhurried",
    "onboarding.mood.calm.greeting": "Take your time. What should we think through first?",
    "onboarding.mood.clearheaded.label": "Clearheaded",
    "onboarding.mood.clearheaded.description": "Crisp, thoughtful, and composed",
    "onboarding.mood.clearheaded.greeting": "What should we make clearer first?",
    "onboarding.mood.quickWitted.label": "Quick-witted",
    "onboarding.mood.quickWitted.description": "Fast, playful, and a little surprising",
    "onboarding.mood.quickWitted.greeting": "All right, what's first?",
    "onboarding.mood.strategic.label": "Strategic",
    "onboarding.mood.strategic.description": "Proactive, practical, and two steps ahead",
    "onboarding.mood.strategic.greeting": "What should we get ahead of first?",

    // Permissions
    "onboarding.permissions.title": "Let Clovy listen and type",
    "onboarding.permissions.subtitle.windows":
      "Dictation and meeting notes need microphone access.",
    "onboarding.permissions.subtitle.other": "Meeting notes need microphone access.",
    "onboarding.permissions.allow": "Allow",
    "onboarding.permissions.microphone.title": "Microphone",
    "onboarding.permissions.microphone.allowAria": "Allow microphone access",
    "onboarding.permissions.microphone.unavailable":
      "No microphone found. Connect one, choose it in Windows sound settings, then try again.",
    "onboarding.permissions.microphone.deniedMac":
      "Turned off in System Settings. Flip the toggle and Clovy will notice.",
    "onboarding.permissions.microphone.deniedWindows":
      "Turned off in Windows settings. Flip the toggle and Clovy will notice.",
    "onboarding.permissions.accessibility.title": "Accessibility",
    "onboarding.permissions.accessibility.allowAria": "Allow accessibility access",
    "onboarding.permissions.systemAudio.title": "System audio",
    "onboarding.permissions.systemAudio.allowAria": "Allow system audio access",
    "onboarding.permissions.systemAudio.denied":
      "Turned off in System Settings. Flip the toggle and Clovy will notice.",
    "onboarding.permissions.systemAudio.unsupported": "Needs macOS 14.2 or later.",
    "onboarding.permissions.systemAudio.unavailable":
      "Allowed. Restart Clovy to finish turning it on.",
    "onboarding.permissions.systemAudio.probing":
      "Waiting for macOS. Approve the prompt when it appears.",
    "onboarding.permissions.work.subtitle":
      "This lets Clovy take meeting notes, hear dictation, and type for you.",
    "onboarding.permissions.work.microphone":
      "Hears your dictation and the meetings you choose to record.",
    "onboarding.permissions.work.accessibility": "Puts your words where you're typing, in any app.",
    "onboarding.permissions.work.systemAudio":
      "Hears the other people on calls so your meeting notes include everyone.",
    "onboarding.permissions.personal.subtitle":
      "This lets Clovy hear voice notes, type for you, and capture a call when you choose.",
    "onboarding.permissions.personal.microphone":
      "Hears voice notes, reflections, and anything you'd rather say than type.",
    "onboarding.permissions.personal.accessibility":
      "Puts dictated messages and notes into other apps.",
    "onboarding.permissions.thinking.subtitle":
      "This lets Clovy hear rough ideas, type drafts, and capture a conversation.",
    "onboarding.permissions.thinking.microphone":
      "Hears you talk through an idea or dictate a draft.",
    "onboarding.permissions.thinking.accessibility":
      "Puts your spoken draft into the app you're working in.",
    "onboarding.permissions.play.subtitle":
      "This lets Clovy hear your ideas, type for you, and capture a role-play.",
    "onboarding.permissions.play.microphone":
      "Hears characters, dialogue, and stories you'd rather speak aloud.",
    "onboarding.permissions.play.accessibility":
      "Puts dialogue and ideas into the app you're using.",
    "onboarding.permissions.captureOnRequest": "Only used when you ask Clovy to capture a call.",
  },
  "pt-BR": {
    "onboarding.progress": "Progresso da configuração: etapa {step} de {total}",
    "onboarding.skipForNow": "Pular por enquanto",

    "onboarding.area.title": "Onde posso ajudar mais?",
    "onboarding.area.subtitle":
      "Escolha a parte da vida que você mais gostaria que eu facilitasse.",
    "onboarding.area.legend": "Escolha onde o Clovy deve ajudar primeiro",
    "onboarding.area.work.label": "Trabalho",
    "onboarding.area.work.description": "Reuniões, acompanhamentos e trabalho focado",
    "onboarding.area.personal.label": "Pessoal",
    "onboarding.area.personal.description": "Planos, diário e o dia a dia",
    "onboarding.area.thinking.label": "Reflexão",
    "onboarding.area.thinking.description": "Ideias, escrita e decisões mais claras",
    "onboarding.area.play.label": "Diversão",
    "onboarding.area.play.description": "Histórias, personagens e projetos criativos",

    "onboarding.mood.title": "Escolha minha personalidade",
    "onboarding.mood.subtitle": "Escolha o clima da nossa primeira conversa.",
    "onboarding.mood.legend": "Escolha o tom de saudação do Clovy",
    "onboarding.mood.saveError": "Ainda não consegui salvar isso. Tente de novo.",
    "onboarding.mood.status": "Tom {mood}. {greeting}",
    "onboarding.mood.characterTitle": "Clovy {mood}",
    "onboarding.mood.calm.label": "Calmo",
    "onboarding.mood.calm.description": "Estável, acolhedor e sem pressa",
    "onboarding.mood.calm.greeting": "Sem pressa. No que vamos pensar primeiro?",
    "onboarding.mood.clearheaded.label": "Lúcido",
    "onboarding.mood.clearheaded.description": "Direto, atencioso e sereno",
    "onboarding.mood.clearheaded.greeting": "O que vamos deixar mais claro primeiro?",
    "onboarding.mood.quickWitted.label": "Espirituoso",
    "onboarding.mood.quickWitted.description": "Rápido, brincalhão e um pouco surpreendente",
    "onboarding.mood.quickWitted.greeting": "Muito bem, o que vem primeiro?",
    "onboarding.mood.strategic.label": "Estratégico",
    "onboarding.mood.strategic.description": "Proativo, prático e dois passos à frente",
    "onboarding.mood.strategic.greeting": "O que vamos antecipar primeiro?",

    "onboarding.permissions.title": "Deixe o Clovy ouvir e digitar",
    "onboarding.permissions.subtitle.windows":
      "O ditado e as notas de reunião precisam de acesso ao microfone.",
    "onboarding.permissions.subtitle.other": "As notas de reunião precisam de acesso ao microfone.",
    "onboarding.permissions.allow": "Permitir",
    "onboarding.permissions.microphone.title": "Microfone",
    "onboarding.permissions.microphone.allowAria": "Permitir acesso ao microfone",
    "onboarding.permissions.microphone.unavailable":
      "Nenhum microfone encontrado. Conecte um, escolha-o nas configurações de som do Windows e tente de novo.",
    "onboarding.permissions.microphone.deniedMac":
      "Desativado nos Ajustes do Sistema. Ative a opção e o Clovy vai perceber.",
    "onboarding.permissions.microphone.deniedWindows":
      "Desativado nas configurações do Windows. Ative a opção e o Clovy vai perceber.",
    "onboarding.permissions.accessibility.title": "Acessibilidade",
    "onboarding.permissions.accessibility.allowAria": "Permitir acesso de acessibilidade",
    "onboarding.permissions.systemAudio.title": "Áudio do sistema",
    "onboarding.permissions.systemAudio.allowAria": "Permitir acesso ao áudio do sistema",
    "onboarding.permissions.systemAudio.denied":
      "Desativado nos Ajustes do Sistema. Ative a opção e o Clovy vai perceber.",
    "onboarding.permissions.systemAudio.unsupported": "Requer macOS 14.2 ou posterior.",
    "onboarding.permissions.systemAudio.unavailable":
      "Permitido. Reinicie o Clovy para terminar de ativar.",
    "onboarding.permissions.systemAudio.probing":
      "Aguardando o macOS. Aprove a solicitação quando ela aparecer.",
    "onboarding.permissions.work.subtitle":
      "Assim o Clovy pode criar notas de reunião, ouvir seu ditado e digitar por você.",
    "onboarding.permissions.work.microphone":
      "Ouve seu ditado e as reuniões que você escolher gravar.",
    "onboarding.permissions.work.accessibility":
      "Coloca suas palavras onde você está digitando, em qualquer app.",
    "onboarding.permissions.work.systemAudio":
      "Ouve as outras pessoas nas chamadas para que suas notas de reunião incluam todos.",
    "onboarding.permissions.personal.subtitle":
      "Assim o Clovy pode ouvir notas de voz, digitar por você e gravar uma chamada quando você quiser.",
    "onboarding.permissions.personal.microphone":
      "Ouve notas de voz, reflexões e tudo o que você prefere falar em vez de digitar.",
    "onboarding.permissions.personal.accessibility":
      "Coloca mensagens e notas ditadas em outros apps.",
    "onboarding.permissions.thinking.subtitle":
      "Assim o Clovy pode ouvir ideias soltas, digitar rascunhos e gravar uma conversa.",
    "onboarding.permissions.thinking.microphone":
      "Ouve você desenvolver uma ideia ou ditar um rascunho.",
    "onboarding.permissions.thinking.accessibility":
      "Coloca seu rascunho falado no app em que você está trabalhando.",
    "onboarding.permissions.play.subtitle":
      "Assim o Clovy pode ouvir suas ideias, digitar por você e gravar um role-play.",
    "onboarding.permissions.play.microphone":
      "Ouve personagens, diálogos e histórias que você prefere contar em voz alta.",
    "onboarding.permissions.play.accessibility":
      "Coloca diálogos e ideias no app que você está usando.",
    "onboarding.permissions.captureOnRequest":
      "Só é usado quando você pede ao Clovy para gravar uma chamada.",
  },
});

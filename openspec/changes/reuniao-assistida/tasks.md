# Tarefas

- [x] Persistir/listar marcações com nota, sessão e tempo ativo; testar isolamento.
- [x] Gerar perguntas com motor Notes sem ferramentas e contexto recente limitado;
  respeitar pause/stop/privacidade e testar falha/resposta atrasada.
- [x] Integrar marcações e assistência ao editor com fontes Você / Outros.
- [x] Verificar no Linux com UI/IPC sintéticos iniciar, duas fontes, marcar,
  pausar/retomar, sugerir, encerrar e reabrir; regressão de reconciliação
  preview/final, isolamento, falhas e mensagens de preservação de áudio.
- [x] Verificar persistência SQLite real, migrações e limites de contexto no
  espelho Rust com fontes idênticas e adaptação isolada do Keychain legado.
- [x] Executar gates disponíveis, revisão e scanner de segredos; registrar
  provas e limitações em .tmp/verify/reuniao-assistida-call-md/worker-report.md.
- [ ] Prova nativa macOS com áudio sintético autorizado, duas fontes,
  permissões, Groq/Parakeet escolhido e motor de Notas real; não comprovada
  pela simulação de navegador ou pelo harness Rust Linux.

export function ChatPattern() {
  return (
    <div className="sg-section">
      <h1 className="sg-section-heading">Chat</h1>
      <p className="sg-section-intro">
        The agent chat turn is the app's richest surface: prose and markdown, tool and reasoning
        disclosures, approval / clarify / secret prompts, generated files and images, and error
        states.
      </p>

      <div className="sg-card">
        <div className="sg-eyebrow">Rendered in the app, not here</div>
        <p className="sg-note" style={{ marginTop: 0 }}>
          The turn renderer (`AgentChatTurnRow`) is an internal, unexported function inside
          `src/components/agent/AgentWorkspace.tsx`, a ~9,800 line module that imports
          `@tauri-apps/api/event` and `@tauri-apps/plugin-dialog` at module scope. Pulling it into
          this dev page would drag its whole graph in and load-time Tauri imports, which the null
          bridge doesn't guard (it only intercepts calls, not imports). So there is no live specimen
          here.
        </p>
        <p className="sg-note">
          To inspect turns, open an agent session in the running app: they render through the real
          `AgentChatTurnRow`, exactly what ships.
        </p>
      </div>
    </div>
  );
}

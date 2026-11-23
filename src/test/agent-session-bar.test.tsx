import userEvent from "@testing-library/user-event";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AgentSessionBar } from "../components/agent/chat-turns/AgentSessionBar";

describe("AgentSessionBar", () => {
  it("keeps core session actions without legacy runtime controls", () => {
    render(
      <AgentSessionBar
        title="Plan the launch"
        onRename={vi.fn()}
        onMoveToProject={vi.fn()}
        onDelete={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Session actions" }));

    expect(screen.getByRole("menuitem", { name: "Rename" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Share" })).not.toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Add to project" })).toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Delete session" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Usage" })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Compact context" })).not.toBeInTheDocument();
    expect(screen.queryByText("Debug with runtime TUI")).not.toBeInTheDocument();
  });
  it("focuses and selects the session title when Rename is activated", async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();
    render(<AgentSessionBar title="Plan the launch" onRename={onRename} />);
    await user.click(screen.getByRole("button", { name: "Session actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    const input = screen.getByRole("textbox", { name: "Session name" });
    expect(input).toHaveFocus();
    expect(input).toHaveProperty("selectionStart", 0);
    expect(input).toHaveProperty("selectionEnd", "Plan the launch".length);
    await user.keyboard("Next launch{Enter}");
    expect(onRename).toHaveBeenCalledExactlyOnceWith("Next launch");
    expect(screen.queryByRole("textbox", { name: "Session name" })).not.toBeInTheDocument();
  });
});

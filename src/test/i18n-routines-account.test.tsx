import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoutineModePicker } from "../components/routines/RoutineModePicker";
import { formatRunTime } from "../components/routines/RoutineRunList";
import { ROUTINE_TEMPLATES } from "../components/routines/routine-templates";
import { SchedulePicker } from "../components/routines/SchedulePicker";
import { applyInterfaceLocale } from "../i18n/locale";

vi.mock("../lib/tauri", () => ({}));

describe("routines and account in Portuguese", () => {
  beforeEach(() => {
    applyInterfaceLocale("pt-BR");
  });
  afterEach(() => applyInterfaceLocale("en"));

  it("names schedule kinds and weekdays in Portuguese", () => {
    render(<SchedulePicker draft={{ kind: "weekly", day: 1, time: "09:00" }} onChange={vi.fn()} />);
    expect(screen.getByText("Semanalmente")).toBeInTheDocument();
    expect(screen.getByText("Segunda-feira")).toBeInTheDocument();
    expect(screen.getByLabelText("Horário")).toBeInTheDocument();
  });

  it("translates the sandbox picker and starter templates", () => {
    render(<RoutineModePicker unrestricted={false} onChange={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Em sandbox" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sem restrições" })).toBeInTheDocument();
    expect(ROUTINE_TEMPLATES[0].name).toBe("Resumo da manhã");
    // Prompts are sent to the model and stay English.
    expect(ROUTINE_TEMPLATES[0].prompt).toMatch(/^Put together a short morning brief/);
  });

  it("interpolates relative run times", () => {
    const now = new Date();
    now.setHours(9, 5, 0, 0);
    expect(formatRunTime(now.toISOString())).toMatch(/^hoje às 0?9:05$/);
  });

  it("switches back to English at runtime", () => {
    act(() => applyInterfaceLocale("en"));
    expect(ROUTINE_TEMPLATES[0].name).toBe("Morning brief");
  });
});

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { VIDEO_MODELS } from "../lib/video-models";
import { ModelPickerDialog, modelOptions } from "../components/settings/ModelPickerDialog";

describe("ModelPickerDialog", () => {
  it("does not offer the retired automatic router", () => {
    const automatic = {
      provider: "open-software",
      id: "open-software/auto",
      name: "Auto",
      modelType: "text",
      traits: [],
      capabilities: [],
    };
    expect(modelOptions([automatic], automatic.id)).toEqual([]);
  });

  it("shows curated video descriptions", () => {
    render(
      <ModelPickerDialog
        open
        mode="video"
        value="wan-2.2-a14b-text-to-video"
        options={VIDEO_MODELS}
        search=""
        onSearchChange={vi.fn()}
        onClose={vi.fn()}
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole("dialog", { name: "Video model" })).toBeInTheDocument();
    expect(
      screen.getByText("Default text-to-video model for fast 5 second 720p clips."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Model details unavailable")).not.toBeInTheDocument();
  });
});

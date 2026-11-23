import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ClovyAlive } from "../components/brand/ClovyAlive";
import {
  CLOVY_LIME,
  CLOVY_LIME_SHADE,
  CLOVY_MARK_PATH,
  CLOVY_WORDMARK_PATHS,
  ClovyMark,
  ClovyWordmark,
} from "../components/brand/ClovyLogo";
import { ProviderLogo } from "../components/settings/ProviderLogo";

describe("ProviderLogo", () => {
  it("uses the standalone Clovy glyph for Auto regardless of its routing provider", () => {
    render(<ProviderLogo provider="openrouter" id="open-software/auto" name="Auto" />);

    const clovyMark = screen.getByRole("img", { name: "Clovy" });
    expect(clovyMark).toBeInTheDocument();
    expect(clovyMark.querySelector("rect")).not.toBeInTheDocument();
    expect(clovyMark.querySelector("path")).toHaveAttribute("fill", "currentColor");
    expect(clovyMark).toHaveAttribute("width", "14");
    expect(clovyMark).toHaveAttribute("height", "14");
    expect(screen.queryByLabelText("OpenSoftware")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("OpenRouter")).not.toBeInTheDocument();
  });

  it("does not classify whisper-large-v3-turbo from provider groq as fal", () => {
    render(
      <ProviderLogo provider="groq" id="whisper-large-v3-turbo" name="Whisper Large V3 Turbo" />,
    );

    expect(screen.queryByLabelText("Fal")).not.toBeInTheDocument();
  });

  it("does not classify broad Anthropic family words as Claude", () => {
    render(<ProviderLogo provider="openrouter" id="poetry-haiku" name="Haiku" />);

    expect(screen.queryByLabelText("Claude")).not.toBeInTheDocument();
  });
  it("classifies xAI providers as Grok", () => {
    render(<ProviderLogo provider="xai" id="grok-4" name="Grok 4" />);

    expect(screen.getByLabelText("Grok")).toBeInTheDocument();
  });
});

describe("Clovy logo", () => {
  it("renders the approved happy-eyes mark with the canonical gradient", () => {
    render(<ClovyMark label="Clovy" />);

    const mark = screen.getByRole("img", { name: "Clovy" });
    expect(mark.querySelector("path")).toHaveAttribute("d", CLOVY_MARK_PATH);
    expect(mark.querySelectorAll("stop")[0]).toHaveAttribute(
      "stop-color",
      `var(--clovy-glow-top, ${CLOVY_LIME})`,
    );
    expect(mark.querySelectorAll("stop")[1]).toHaveAttribute(
      "stop-color",
      `var(--clovy-glow, ${CLOVY_LIME_SHADE})`,
    );
  });

  it("renders all exact wordmark paths", () => {
    render(<ClovyWordmark label="Clovy wordmark" variant="mono" />);

    const wordmark = screen.getByRole("img", { name: "Clovy wordmark" });
    expect(Array.from(wordmark.querySelectorAll("path"), (path) => path.getAttribute("d"))).toEqual(
      CLOVY_WORDMARK_PATHS,
    );
  });

  it("gives the living Home character the marketing material and independent eyes", () => {
    render(<ClovyAlive label="Living Clovy" />);

    const character = screen.getByRole("img", { name: "Living Clovy" });
    expect(character).toHaveAttribute("data-palette", "identity");
    expect(character.querySelector(".clovy-alive-sheen")).toBeInTheDocument();
    expect(character.querySelectorAll(".clovy-alive-eye")).toHaveLength(2);
    expect(character.querySelector(".clovy-alive-body")).toBeInTheDocument();
  });

  it("allows the Home character to opt into the Appearance palette", () => {
    render(<ClovyAlive label="Themed Clovy" palette="appearance" />);

    expect(screen.getByRole("img", { name: "Themed Clovy" })).toHaveAttribute(
      "data-palette",
      "appearance",
    );
  });
});

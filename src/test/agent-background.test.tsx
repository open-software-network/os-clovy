import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentBackground } from "../components/agent/AgentBackground";
import { setStoredAppBackground } from "../lib/app-background";

const imageApi = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => imageApi);

const kit = vi.hoisted(() => ({ background: vi.fn(), drift: { id: "drift" } }));
vi.mock("asciify-engine/kit", () => ({ background: kit.background, drift: kit.drift }));

function Surface() {
  return (
    <section>
      <AgentBackground />
      <div>Content</div>
    </section>
  );
}

describe("agent background lifecycle", () => {
  const handles: { destroy: ReturnType<typeof vi.fn> }[] = [];
  beforeEach(() => {
    localStorage.clear();
    imageApi.invoke.mockReset().mockResolvedValue(null);
    handles.length = 0;
    kit.background.mockReset().mockImplementation((host: HTMLElement) => {
      const canvas = document.createElement("canvas");
      host.append(canvas);
      const handle = { destroy: vi.fn(() => canvas.remove()) };
      handles.push(handle);
      return handle;
    });
  });
  afterEach(cleanup);

  it("uses the selected Drift controls across the entire surface", async () => {
    const view = render(<Surface />);
    await waitFor(() => expect(kit.background).toHaveBeenCalledOnce());
    const [host, options] = kit.background.mock.calls[0];
    expect(host.getAttribute("aria-hidden")).toBe("true");
    expect(options).toMatchObject({
      preset: kit.drift,
      mark: "dot",
      ink: "--ascii-ink",
      speed: 1,
      cellSize: 11,
      opacity: 0.14,
      cursor: "trail",
      look: { scene: "flow", contrast: 1.4, scale: 1.6 },
    });
    expect(options).not.toHaveProperty("clear");
    view.rerender(<Surface />);
    expect(kit.background).toHaveBeenCalledOnce();
    view.unmount();
    expect(handles[0].destroy).toHaveBeenCalledOnce();
  });

  it("switches off live and mounts only one new renderer when switched back", async () => {
    const view = render(<Surface />);
    await waitFor(() => expect(view.container.querySelectorAll("canvas")).toHaveLength(1));
    act(() => setStoredAppBackground("none"));
    expect(view.container.querySelectorAll("canvas")).toHaveLength(0);
    expect(handles[0].destroy).toHaveBeenCalledOnce();
    act(() => setStoredAppBackground("drift"));
    await waitFor(() => expect(view.container.querySelectorAll("canvas")).toHaveLength(1));
    expect(handles).toHaveLength(2);
    view.unmount();
    expect(handles[1].destroy).toHaveBeenCalledOnce();
  });

  it("does not load a renderer for a saved None preference", async () => {
    setStoredAppBackground("none");
    const view = render(<Surface />);
    await act(async () => {});
    expect(kit.background).not.toHaveBeenCalled();
    expect(view.container.querySelector(".agent-background")).toBeNull();
  });

  it("leaves content usable when initialization fails", async () => {
    kit.background.mockImplementation(() => {
      throw new Error("WebGL unavailable");
    });
    const view = render(<Surface />);
    await waitFor(() => expect(kit.background).toHaveBeenCalledOnce());
    expect(view.getByText("Content")).toBeVisible();
    expect(view.container.querySelector("canvas")).toBeNull();
  });

  it("replaces Drift with a decorative image and goes plain on decode failure", async () => {
    imageApi.invoke.mockResolvedValue({
      fileName: "lake.png",
      dataUrl: "data:image/png;base64,AA==",
    });
    const view = render(<AgentBackground />);
    await waitFor(() => expect(view.container.querySelector("canvas")).not.toBeNull());
    const handle = kit.background.mock.results.at(-1)?.value;
    act(() => setStoredAppBackground("image"));
    await waitFor(() => expect(view.container.querySelector("img")).not.toBeNull());
    expect(handle.destroy).toHaveBeenCalledOnce();
    expect(view.container.querySelector("canvas")).toBeNull();
    const img = view.container.querySelector("img") as HTMLImageElement;
    expect(img.alt).toBe("");
    expect(img.parentElement?.getAttribute("aria-hidden")).toBe("true");
    fireEvent.error(img);
    expect(view.container.querySelector("img")).toBeNull();
    view.unmount();
  });
  it("renders plain when the saved image is missing", async () => {
    setStoredAppBackground("image");
    imageApi.invoke.mockResolvedValue({ fileName: "missing.png", dataUrl: null });
    const view = render(<AgentBackground />);
    await waitFor(() => expect(imageApi.invoke).toHaveBeenCalledWith("read_background_image"));
    expect(view.container.innerHTML).toBe("");
    view.unmount();
  });
});

import {
  Component,
  type ComponentProps,
  type ComponentType,
  createElement,
  lazy,
  type ReactNode,
  Suspense,
  useMemo,
  useState,
} from "react";
import { useT } from "../i18n";

type WorkspaceLoader<Props extends object> = {
  Component: ComponentType<Props>;
  preload: () => Promise<void>;
};

export function createWorkspaceLoader<Module, Props extends object>(
  loadModule: () => Promise<Module>,
  selectComponent: (module: Module) => ComponentType<Props>,
): WorkspaceLoader<Props> {
  let modulePromise: Promise<Module> | undefined;

  function load() {
    modulePromise ??= Promise.resolve()
      .then(loadModule)
      .catch((error: unknown) => {
        modulePromise = undefined;
        throw error;
      });
    return modulePromise;
  }

  function createLazyComponent() {
    return lazy(async () => ({
      default: selectComponent(await load()),
    }));
  }

  const InitialLazyComponent = createLazyComponent();

  function WorkspaceRoute(props: Props) {
    const [attempt, setAttempt] = useState(0);
    const LazyComponent = useMemo(
      () => (attempt === 0 ? InitialLazyComponent : createLazyComponent()),
      [attempt],
    );
    const ComponentToRender = LazyComponent as unknown as ComponentType<Props>;

    return (
      <WorkspaceLoadErrorBoundary key={attempt} onRetry={() => setAttempt((value) => value + 1)}>
        <Suspense fallback={<WorkspaceFallback />}>
          {createElement(ComponentToRender, props)}
        </Suspense>
      </WorkspaceLoadErrorBoundary>
    );
  }

  return {
    Component: WorkspaceRoute,
    preload: async () => {
      await load();
    },
  };
}

type WorkspaceLoadErrorBoundaryProps = {
  children: ReactNode;
  onRetry: () => void;
};

class WorkspaceLoadErrorBoundary extends Component<
  WorkspaceLoadErrorBoundaryProps,
  { failed: boolean }
> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    if (this.state.failed) {
      return <WorkspaceLoadFailure onRetry={this.props.onRetry} />;
    }
    return this.props.children;
  }
}

function WorkspaceLoadFailure({ onRetry }: { onRetry: () => void }) {
  const t = useT();
  return (
    <section className="workspace-fallback workspace-load-error" role="alert">
      <h2>{t("app.workspace.loadFailedTitle")}</h2>
      <p>{t("app.workspace.loadFailedBody")}</p>
      <button type="button" className="primary-action primary-solid" onClick={onRetry}>
        {t("common.tryAgain")}
      </button>
    </section>
  );
}

type FoldersWorkspaceModule = typeof import("../components/folders/FoldersWorkspace");
type FoldersWorkspaceProps = ComponentProps<FoldersWorkspaceModule["FoldersWorkspace"]>;
type NoteEditorModule = typeof import("../components/note-editor/NoteEditor");
type NoteEditorProps = ComponentProps<NoteEditorModule["NoteEditor"]>;
type RoutinesViewModule = typeof import("../components/routines/RoutinesView");
type RoutinesViewProps = Record<string, never>;
type AppSettingsModule = typeof import("../components/settings/AppSettings");
type AppSettingsProps = ComponentProps<AppSettingsModule["AppSettings"]>;
type ActivityTimelineViewModule =
  typeof import("../components/activity-timeline/ActivityTimelineView");
type ActivityTimelineViewProps = ComponentProps<ActivityTimelineViewModule["ActivityTimelineView"]>;

const foldersWorkspace = createWorkspaceLoader<FoldersWorkspaceModule, FoldersWorkspaceProps>(
  () => import("../components/folders/FoldersWorkspace"),
  (module) => module.FoldersWorkspace,
);
const noteEditor = createWorkspaceLoader<NoteEditorModule, NoteEditorProps>(
  () => import("../components/note-editor/NoteEditor"),
  (module) => module.NoteEditor,
);
const routinesView = createWorkspaceLoader<RoutinesViewModule, RoutinesViewProps>(
  () => import("../components/routines/RoutinesView"),
  (module) => module.RoutinesView,
);
const appSettings = createWorkspaceLoader<AppSettingsModule, AppSettingsProps>(
  () => import("../components/settings/AppSettings"),
  (module) => module.AppSettings,
);
const activityTimelineView = createWorkspaceLoader<
  ActivityTimelineViewModule,
  ActivityTimelineViewProps
>(
  () => import("../components/activity-timeline/ActivityTimelineView"),
  (module) => module.ActivityTimelineView,
);

export const FoldersWorkspaceRoute = foldersWorkspace.Component;
export const NoteEditorRoute = noteEditor.Component;
export const RoutinesViewRoute = routinesView.Component;
export const AppSettingsRoute = appSettings.Component;
export const ActivityTimelineViewRoute = activityTimelineView.Component;

const deferredWorkspacePreloads = [
  // Meeting-start navigation lands here, so queue the note editor first.
  noteEditor.preload,
  routinesView.preload,
  appSettings.preload,
  foldersWorkspace.preload,
  activityTimelineView.preload,
];

type IdleCallbackWindow = Window & {
  requestIdleCallback?: (callback: () => void, options?: { timeout: number }) => number;
  cancelIdleCallback?: (handle: number) => void;
};

export function prefetchRemainingWorkspacesAfterPaint() {
  const idleWindow = window as IdleCallbackWindow;
  let cancelled = false;
  let idleHandle: number | undefined;
  let timeoutHandle: number | undefined;

  const runPrefetch = () => {
    if (cancelled) return;
    // Failed idle imports reset in load() and remain retryable on navigation.
    void Promise.allSettled(deferredWorkspacePreloads.map((preload) => preload()));
  };

  const frameHandle = window.requestAnimationFrame(() => {
    if (cancelled) return;
    if (idleWindow.requestIdleCallback) {
      idleHandle = idleWindow.requestIdleCallback(runPrefetch, { timeout: 1_500 });
      return;
    }
    timeoutHandle = window.setTimeout(runPrefetch, 0);
  });

  return () => {
    cancelled = true;
    window.cancelAnimationFrame(frameHandle);
    if (idleHandle !== undefined) idleWindow.cancelIdleCallback?.(idleHandle);
    if (timeoutHandle !== undefined) window.clearTimeout(timeoutHandle);
  };
}

function WorkspaceFallback() {
  const t = useT();
  return (
    <section
      className="workspace-fallback"
      aria-label={t("app.workspace.loading")}
      aria-busy="true"
    >
      <span className="workspace-fallback-title" />
      <div className="workspace-fallback-lines" aria-hidden="true">
        <span />
        <span />
        <span />
      </div>
    </section>
  );
}

export const preloadInitialWorkspace = activityTimelineView.preload;

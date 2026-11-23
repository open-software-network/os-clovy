import { LocalTranscriptionSettings } from "./LocalTranscriptionSettings";
import { listen } from "@tauri-apps/api/event";
import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconCircleCheck } from "central-icons/IconCircleCheck";
import { IconCircleQuestionmark } from "central-icons/IconCircleQuestionmark";
import { IconCircleX } from "central-icons/IconCircleX";
import { IconExclamationCircle } from "central-icons/IconExclamationCircle";
import { IconMoonStar } from "central-icons/IconMoonStar";
import { IconSun } from "central-icons/IconSun";
import { IconTelevision } from "central-icons/IconTelevision";
import { useEffect, useRef, useState } from "react";
import type { CSSProperties, ReactNode, RefObject } from "react";
import {
  INTERFACE_LOCALE_OPTIONS,
  type InterfaceLocale,
  type MessageKey,
  setInterfaceLocale,
  t as translate,
  type TFunction,
  useLocale,
  useT,
} from "../../i18n";
import {
  appPreferencesGet,
  appPreferencesSet,
  dictationHotkeyStatus,
  dictationHelperCommand,
  dictationSettings,
  listProviderModels,
  localAudioFileSrc,
  providerModelSettings,
  clearProviderApiKey,
  setDictationLanguage,
  setDictationMicrophone,
  setDictationShortcut,
  setImageSafeMode,
  setLiveTranscription,
  setProviderApiKey,
  setProviderModel,
} from "../../lib/tauri";
import { LANGUAGE_OPTIONS, languageLabel } from "../../lib/dictation-languages";
import { autostartEnabled, autostartSupported, setAutostartEnabled } from "../../lib/autostart";
import { replayOnboarding } from "../../lib/onboarding";
import { errorCode, messageForErrorCode } from "../../lib/errors";
import type {
  DictationHelperEvent,
  DictationMicrophoneDeviceDto,
  DictationShortcutKind,
  DictationSettingsDto,
  DictationShortcutModifiers,
  DictationShortcutSetting,
  BuiltinProvider,
  FolderDto,
  ProviderModelDto,
  ProviderModelMode,
  ProviderModelSettingsDto,
  RecordingSourceMode,
  RecordingSourceReadinessDto,
} from "../../lib/tauri";
import { KeycapShortcut } from "../shortcuts/KeycapShortcut";
import { chordFromKeyEvent, shortcutFromCapturePayload } from "../shortcuts/use-shortcut-capture";
import {
  Select,
  selectPopoverPlacement,
  selectPopoverStyle,
  type SelectPopoverPlacement,
} from "../ui/Select";
import { SegmentedControl } from "../ui/SegmentedControl";
import { InlineNotice } from "../ui/InlineNotice";
import { Switch } from "../ui/Switch";
import { HoverTip } from "../ui/HoverTip";
import { toast } from "../ui/Toaster";
import { APP_COMMIT_HASH, APP_VERSION } from "../../app/build-info";
import { getStoredTheme, setStoredTheme, type ThemePreference } from "../../lib/theme";
import { BRAND_PRESETS, getStoredBrand, setStoredBrand, type BrandId } from "../../lib/brand";
import {
  FONT_SCALE_PRESETS,
  setStoredFontScale,
  useFontScaleId,
  type FontScaleId,
} from "../../lib/font-scale";
import {
  fallbackDictationCapabilities,
  isMacLikePlatform,
  isSystemAudioSupportedPlatform,
  isWindowsPlatform,
  useDictationCapabilities,
} from "../../lib/platform";
import { systemAudioAvailability } from "../../lib/source-readiness";
import { parseDictationHelperEvent } from "../../lib/dictation-events";
import { dispatchProviderModelSettingsChanged } from "../../lib/model-privacy";
import type { LlmProvidersDto } from "../../lib/llm-providers";
import { LlmProvidersSection } from "./LlmProvidersSection";
import { ProviderLogo } from "./ProviderLogo";
import { modelOptions, selectedModel } from "./ModelPickerDialog";
import { DEFAULT_GENERATION_SUGGESTION_ID } from "../../lib/suggested-models";
import {
  ModelPickerCardContent,
  ModelPickerPopover,
  type ModelPickerFlyout,
} from "./ModelPickerPopover";
import { DEFAULT_IMAGE_MODEL, imageModelCatalog } from "../../lib/image-models";
import { IMAGE_GENERATION_ENABLED, VIDEO_GENERATION_ENABLED } from "../../lib/feature-flags";
import {
  INITIAL_EXPERIMENTAL_UNLOCK_CLICK_STATE,
  registerExperimentalUnlockClick,
  setExperimentalFlags,
  useExperimentalFlags,
} from "../../lib/experimental-flags";
import { DEFAULT_VIDEO_MODEL, videoModelCatalog } from "../../lib/video-models";
import { AgentSettingsSection } from "./AgentSettingsSection";
import { McpServerSection } from "./McpServerSection";
import { AgentMcpServersSection } from "./AgentMcpServersSection";
import { ConnectorsSection } from "./ConnectorsSection";
import { DictionarySettingsSection } from "./DictionarySettingsSection";
import { MemorySettingsSection } from "./MemorySettingsSection";
import { MicTestControl, type MicTestState } from "./MicTestControl";
import { StyleSettingsSection } from "./StyleSettingsSection";
import { ActivitySettingsSection } from "./ActivitySettingsSection";
import { activityStatus } from "../../lib/activity-capture";
import { SETTINGS_TABS, settingsTabsForPlatform, type SettingsTab } from "./settings-config";
import { DEFAULT_DATA_PARTITION, useCurrentDataPartitionName } from "../../lib/data-partition";
function themeOptions(t: TFunction): readonly {
  value: ThemePreference;
  label: ReactNode;
  ariaLabel: string;
}[] {
  return [
    {
      value: "system",
      label: (
        <>
          <IconTelevision size={14} />
          {t("settings.theme.system")}
        </>
      ),
      ariaLabel: t("settings.theme.systemAria"),
    },
    {
      value: "light",
      label: (
        <>
          <IconSun size={14} />
          {t("settings.theme.light")}
        </>
      ),
      ariaLabel: t("settings.theme.lightAria"),
    },
    {
      value: "dark",
      label: (
        <>
          <IconMoonStar size={14} />
          {t("settings.theme.dark")}
        </>
      ),
      ariaLabel: t("settings.theme.darkAria"),
    },
  ];
}

const FONT_SCALE_LABEL_KEYS = {
  default: "settings.textSize.default",
  large: "settings.textSize.large",
  larger: "settings.textSize.larger",
} as const satisfies Record<FontScaleId, MessageKey>;

function fontScaleOptions(t: TFunction): readonly {
  value: FontScaleId;
  label: ReactNode;
  ariaLabel: string;
}[] {
  return FONT_SCALE_PRESETS.map((preset) => {
    const label = t(FONT_SCALE_LABEL_KEYS[preset.id]);
    return {
      value: preset.id,
      label,
      ariaLabel: t("settings.textSize.optionAria", { size: label }),
    };
  });
}

const EMPTY_MODIFIERS: DictationShortcutModifiers = {
  command: false,
  control: false,
  option: false,
  shift: false,
  function: false,
};

const DEFAULT_SETTINGS: DictationSettingsDto = {
  pushToTalkShortcut: {
    keyCode: 0x02,
    code: "KeyD",
    label: "Ctrl+Opt+D",
    pressCount: 1,
    modifiers: {
      ...EMPTY_MODIFIERS,
      control: true,
      option: true,
    },
  },
  toggleShortcut: {
    keyCode: 0x11,
    code: "KeyT",
    label: "Ctrl+Opt+T",
    pressCount: 1,
    modifiers: {
      ...EMPTY_MODIFIERS,
      control: true,
      option: true,
    },
  },
  microphone: {},
  style: "standard",
  language: undefined,
  cleanupEnabled: false,
  cleanupModel: "nvidia/nemotron-3-nano-30b-a3b",
  hudPlacement: "bottom",
};

// Windows has no Fn: dictation defaults to Right Ctrl alone (mirrors
// `DictationSettings::default_for_platform` in Rust).
const WINDOWS_DEFAULT_SHORTCUTS: Record<DictationShortcutKind, DictationShortcutSetting> = {
  push_to_talk: {
    keyCode: 0,
    code: "ControlRight",
    modifiers: { command: false, control: false, option: false, shift: false, function: false },
    label: "Right Ctrl",
    pressCount: 1,
  },
  toggle: {
    ...DEFAULT_SETTINGS.toggleShortcut,
    label: "Ctrl+Alt+T",
  },
};

const DEFAULT_PROVIDER_MODELS: ProviderModelSettingsDto = {
  transcriptionProvider: "groq",
  generationProvider: "openrouter",
  transcriptionModel: "whisper-large-v3-turbo",
  // The factory default text model: DEFAULT_GENERATION_MODEL in the Rust
  // providers module, mirrored by the leading Suggested pick.
  generationModel: DEFAULT_GENERATION_SUGGESTION_ID,
  remoteGenerationModel: DEFAULT_GENERATION_SUGGESTION_ID,
  imageModel: DEFAULT_IMAGE_MODEL,
  videoModel: DEFAULT_VIDEO_MODEL,
  providerApiKeysConfigured: {
    groq: false,
    openrouter: false,
    fal: false,
    tavily: false,
  },
  localGeneration: {
    baseUrl: "",
    modelId: "",
  },
  // On by default, matching the Rust providers default.
  imageSafeMode: true,
  imageSafeModePromptDismissed: false,
  liveTranscription: true,
};

type ProviderModelSettingsSnapshot = {
  settings: ProviderModelSettingsDto;
  effectiveSettings?: ProviderModelSettingsDto;
};

function mergeProviderModelSettings(settings?: Partial<ProviderModelSettingsDto>) {
  return {
    ...DEFAULT_PROVIDER_MODELS,
    ...settings,
  };
}

function providerModelSettingsSnapshot(response: ProviderModelSettingsSnapshot) {
  const settings = mergeProviderModelSettings(response.settings);
  return {
    settings,
    effectiveSettings: mergeProviderModelSettings(response.effectiveSettings ?? response.settings),
  };
}

const MIC_TEST_DURATION_SECONDS = 5;

export type { SettingsTab };
// Same list (and translated `label` getters) as the sidebar's settings nav.
export { SETTINGS_TABS };

/**
 * The shared settings page header (Codex-app style): a large serif page title
 * with one muted one-line blurb beneath, and generous space before the content.
 * Every settings panel opens with this so the page announces what it is; panels
 * with multiple sub-groups keep their small `.settings-group-heading` labels
 * below it.
 */
export function SettingsPageHeader({
  id,
  title,
  blurb,
}: {
  /** Ties the panel's `aria-labelledby` to the visible page title. */
  id?: string;
  title: ReactNode;
  blurb?: ReactNode;
}) {
  return (
    <header className="settings-page-header">
      <h2 id={id} className="settings-page-title">
        {title}
      </h2>
      {blurb ? <p className="settings-page-blurb">{blurb}</p> : null}
    </header>
  );
}

type AppSettingsProps = {
  folders?: FolderDto[];
  onFoldersImported?: (folders: FolderDto[]) => void;
  /** When Memory is opened from a project, the manager pre-filters to it. */
  memoryFolderFilter?: string;
  /** Drill from a memory's project tag into that project. */
  onOpenProject?: (folderId: string) => void;
  sourceMode: RecordingSourceMode;
  sourceReadiness?: RecordingSourceReadinessDto;
  checkingSourceReadiness: boolean;
  microphonePermissionStatus?: string;
  accessibilityPermissionStatus?: string;
  onSourceModeChange: (mode: RecordingSourceMode) => void;
  onEnableMicrophone?: () => void;
  onEnableAccessibility?: () => void;
  onEnableSystemAudio: () => void;
  // When the host (the sidebar settings nav) drives the active section, it
  // passes both of these so AppSettings becomes a controlled panel and hides
  // its own header + in-page tab nav. Left undefined, AppSettings keeps its
  // own nav — the standalone path exercised by app-settings tests.
  activeTab?: SettingsTab;
  onTabChange?: (tab: SettingsTab) => void;
  // Reports when a drill-in detail (skill detail) is open so the host can hand
  // this view the scroll container, notes-style: the shell sets
  // data-note-detail-scroller on .main-panel-body and the detail owns its own
  // scroll region under a pinned breadcrumb bar.
  onDetailPinnedChange?: (pinned: boolean) => void;
  activitySupported?: boolean;
  // Runs the updater's manual check. Absent in a development build, which
  // never checks, so the Updates section does not render.
  onCheckForUpdates?: () => void;
  checkingForUpdates?: boolean;
};

export function AppSettings({
  folders = [],
  onFoldersImported,
  memoryFolderFilter,
  onOpenProject,
  sourceMode,
  sourceReadiness,
  checkingSourceReadiness,
  microphonePermissionStatus,
  accessibilityPermissionStatus,
  onSourceModeChange,
  onEnableMicrophone,
  onEnableAccessibility,
  onEnableSystemAudio,
  activeTab: controlledTab,
  onTabChange,
  onDetailPinnedChange,
  onCheckForUpdates,
  checkingForUpdates = false,
  activitySupported: activitySupportedProp,
}: AppSettingsProps) {
  const [settings, setSettings] = useState<DictationSettingsDto>(DEFAULT_SETTINGS);
  const [providerSettings, setProviderSettings] =
    useState<ProviderModelSettingsDto>(DEFAULT_PROVIDER_MODELS);
  const [effectiveProviderSettings, setEffectiveProviderSettings] =
    useState<ProviderModelSettingsDto>(DEFAULT_PROVIDER_MODELS);
  const providerSettingsProfileRef = useRef<string | null>(null);
  const currentDataPartitionLabel = useCurrentDataPartitionName();
  const showingPartitionModels = currentDataPartitionLabel !== DEFAULT_DATA_PARTITION;
  const [partitionGenerationModel, setPartitionGenerationModel] = useState<string>();
  const [providerModels, setProviderModels] = useState<
    Record<ProviderModelMode, ProviderModelDto[]>
  >({
    transcription: [],
    generation: [],
    image: [],
    video: [],
  });
  const [microphones, setMicrophones] = useState<DictationMicrophoneDeviceDto[]>([]);
  const [defaultMicrophone, setDefaultMicrophone] = useState<DictationMicrophoneDeviceDto>();
  const [capturingShortcut, setCapturingShortcut] = useState<DictationShortcutKind>();
  const capturingShortcutRef = useRef<DictationShortcutKind>();
  const [shortcutError, setShortcutError] = useState<string>();
  const [shortcutErrorKind, setShortcutErrorKind] = useState<DictationShortcutKind>();
  const [status, setStatus] = useState<string>();
  // Set when the dictation helper dies (crash or the bundle swap after an
  // update) and cleared once it re-arms the hotkey, so the shortcuts pane never
  // silently shows a dead hotkey.
  const [helperUnavailable, setHelperUnavailable] = useState<{
    reason: string;
    message: string;
  }>();
  const [micOpen, setMicOpen] = useState(false);
  const t = useT();
  const interfaceLocale = useLocale();
  const [theme, setTheme] = useState<ThemePreference>(() => getStoredTheme());
  const [brand, setBrand] = useState<BrandId>(() => getStoredBrand());
  const fontScale = useFontScaleId();
  const experimentalFlags = useExperimentalFlags();
  const experimentalUnlockClicksRef = useRef({
    ...INITIAL_EXPERIMENTAL_UNLOCK_CLICK_STATE,
  });
  const experimentalUnlockingRef = useRef(false);
  const [experimentalOperation, setExperimentalOperation] = useState<"flags">();
  const [experimentalError, setExperimentalError] = useState<string>();
  const [pickerMode, setPickerMode] = useState<ProviderModelMode>();
  const [modelPickerFlyout, setModelPickerFlyout] = useState<ModelPickerFlyout>(null);
  const [modelSearch, setModelSearch] = useState("");
  const modelPickerTriggerRef = useRef<HTMLButtonElement>(null);
  const modelPickerPopoverRef = useRef<HTMLDivElement>(null);
  const modelPickerSearchRef = useRef<HTMLInputElement>(null);
  const [providerApiKeyDrafts, setProviderApiKeyDrafts] = useState<Record<BuiltinProvider, string>>(
    {
      groq: "",
      openrouter: "",
      fal: "",
      tavily: "",
    },
  );
  const [providerApiKeyRejectedErrors, setProviderApiKeyRejectedErrors] = useState<
    Record<BuiltinProvider, string | undefined>
  >({
    groq: undefined,
    openrouter: undefined,
    fal: undefined,
    tavily: undefined,
  });
  const [notesProviders, setNotesProviders] = useState<LlmProvidersDto | null>(null);
  const [generationCatalogFailed, setGenerationCatalogFailed] = useState(false);
  const [showMoreVoiceOptions, setShowMoreVoiceOptions] = useState(false);
  const [showMoreTextOptions, setShowMoreTextOptions] = useState(false);
  const [showMoreImageOptions, setShowMoreImageOptions] = useState(false);
  const [internalTab, setInternalTab] = useState<SettingsTab>("general");
  const [micPopoverPlacement, setMicPopoverPlacement] =
    useState<SelectPopoverPlacement>("align-selected");
  const [languageOpen, setLanguageOpen] = useState(false);
  const [languagePopoverPlacement, setLanguagePopoverPlacement] =
    useState<SelectPopoverPlacement>("align-selected");
  const [micTestState, setMicTestState] = useState<MicTestState>("idle");
  const [micTestLevel, setMicTestLevel] = useState(0);
  const [micTestStartedAt, setMicTestStartedAt] = useState<number>();
  const [micTestSampleSrc, setMicTestSampleSrc] = useState<string>();

  const [micTestError, setMicTestError] = useState<string>();
  const [micTestPlaying, setMicTestPlaying] = useState(false);
  const controlled = controlledTab !== undefined && onTabChange !== undefined;
  const activeTab = controlled ? controlledTab : internalTab;
  // The skill opened from Installed skills. While set (and the skills tab is
  // active) the whole settings page swaps for the notes-style detail shell:
  // pinned breadcrumb bar on top, its own scroll region beneath.
  const detailPinned = false;
  useEffect(() => {
    onDetailPinnedChange?.(detailPinned);
  }, [detailPinned, onDetailPinnedChange]);
  // Never leave the host thinking a detail scroller is active after unmount.
  useEffect(() => () => onDetailPinnedChange?.(false), [onDetailPinnedChange]);
  const [internalActivitySupported, setInternalActivitySupported] = useState(
    () => isMacLikePlatform() || isWindowsPlatform(),
  );
  useEffect(() => {
    if (activitySupportedProp !== undefined) return;
    let active = true;
    void activityStatus()
      .then((status) => {
        if (active) setInternalActivitySupported(status.supported);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [activitySupportedProp]);
  const effectiveActivitySupported = activitySupportedProp ?? internalActivitySupported;
  const settingsTabs = settingsTabsForPlatform(effectiveActivitySupported);
  const capabilities = useDictationCapabilities();
  const systemAudioSupportedPlatform = capabilities.systemAudio || isSystemAudioSupportedPlatform();
  const defaultShortcuts =
    capabilities.platform === "windows"
      ? WINDOWS_DEFAULT_SHORTCUTS
      : {
          push_to_talk: DEFAULT_SETTINGS.pushToTalkShortcut,
          toggle: DEFAULT_SETTINGS.toggleShortcut,
        };
  const modifierRequiredMessage =
    capabilities.platform === "windows"
      ? t("settings.shortcuts.modifierRequiredWindows")
      : t("settings.shortcuts.modifierRequiredMac");

  const setActiveTab = (tab: SettingsTab) => {
    if (controlled) {
      onTabChange?.(tab);
    } else {
      setInternalTab(tab);
    }
  };
  const micWrapRef = useRef<HTMLDivElement>(null);
  const languageWrapRef = useRef<HTMLDivElement>(null);
  const systemOn = sourceMode === "microphonePlusSystem";
  const systemReadiness = sourceReadiness?.sources.find((source) => source.source === "system");
  const microphoneReadiness = sourceReadiness?.sources.find(
    (source) => source.source === "microphone",
  );
  const systemAvailability = systemAudioAvailability(sourceReadiness);
  // Denied and granted-but-uncapturable both lock the switch, but only a real
  // denial is fixable in System Settings: the uncapturable helper recovers on
  // restart, so sending the user to grant an already-granted permission would
  // be a dead end. The status label tells the two apart.
  const systemDenied = systemAvailability === "denied";
  const systemLocked = systemDenied || systemAvailability === "unavailable";
  const systemUnavailable = !systemAudioSupportedPlatform || systemAvailability === "unsupported";

  useEffect(() => {
    capturingShortcutRef.current = capturingShortcut;
  }, [capturingShortcut]);

  useEffect(() => {
    setMicOpen(false);
    setLanguageOpen(false);
    if (activeTab !== "audio" && micTestState !== "idle") {
      void resetMicTestState(true);
    }
  }, [activeTab]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    async function boot() {
      try {
        const response = await dictationSettings();
        if (cancelled) return;
        setSettings(response.settings);
        const hotkeyStatus = await dictationHotkeyStatus();
        if (cancelled) return;
        handleHelperEvent(hotkeyStatus);
        const modelResponse = await providerModelSettings();
        if (cancelled) return;
        // Merge over defaults so a settings payload that predates a field
        // (e.g. imageModel from an older backend) still has every model set.
        const modelSnapshot = providerModelSettingsSnapshot(modelResponse);
        setProviderSettings(modelSnapshot.settings);
        setEffectiveProviderSettings(modelSnapshot.effectiveSettings);
        providerSettingsProfileRef.current = currentDataPartitionLabel;
        await requestMicrophones();
        await Promise.all([
          requestProviderModels("transcription"),
          requestProviderModels("generation"),
          requestProviderModels("image"),
          requestProviderModels("video"),
        ]);
      } catch (error) {
        if (!cancelled) setStatus(messageFromError(error));
      }
    }

    void listen<string>("dictation-event", (event) => {
      const helperEvent = parseDictationHelperEvent(event.payload);
      if (helperEvent) handleHelperEvent(helperEvent);
    }).then((cleanup) => {
      // Unmount can race the listen() promise — unsubscribe immediately
      // instead of leaking the listener.
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    void boot();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (
      providerSettingsProfileRef.current === null ||
      providerSettingsProfileRef.current === currentDataPartitionLabel
    ) {
      return;
    }
    let cancelled = false;

    async function refreshProviderModelsForProfile() {
      try {
        const modelResponse = await providerModelSettings();
        if (cancelled) return;
        const modelSnapshot = providerModelSettingsSnapshot(modelResponse);
        setProviderSettings(modelSnapshot.settings);
        setEffectiveProviderSettings(modelSnapshot.effectiveSettings);
        providerSettingsProfileRef.current = currentDataPartitionLabel;
      } catch (error) {
        if (!cancelled) setStatus(messageFromError(error));
      }
    }

    void refreshProviderModelsForProfile();
    return () => {
      cancelled = true;
    };
  }, [currentDataPartitionLabel]);

  useEffect(() => {
    if (!showingPartitionModels) {
      setPartitionGenerationModel(undefined);
      return;
    }
    let cancelled = false;
    setPartitionGenerationModel(undefined);

    async function loadActiveProfileTextModel() {
      try {
        if (!cancelled) setPartitionGenerationModel(effectiveProviderSettings.generationModel);
      } catch {
        if (!cancelled) setPartitionGenerationModel(undefined);
      }
    }

    void loadActiveProfileTextModel();
    return () => {
      cancelled = true;
    };
  }, [effectiveProviderSettings.generationModel, showingPartitionModels]);

  useEffect(() => {
    if (!micOpen) return;
    function onPointer(event: MouseEvent) {
      if (!micWrapRef.current?.contains(event.target as Node)) {
        setMicOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setMicOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [micOpen]);

  useEffect(() => {
    if (!languageOpen) return;
    function onPointer(event: MouseEvent) {
      if (!languageWrapRef.current?.contains(event.target as Node)) {
        setLanguageOpen(false);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setLanguageOpen(false);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [languageOpen]);

  // The capture effect below must call the latest saveShortcut, not the one
  // from the render in which capturing began (it is a plain function,
  // redefined every render). Same ref pattern as use-shortcut-capture.
  const saveShortcutRef = useRef(saveShortcut);
  useEffect(() => {
    saveShortcutRef.current = saveShortcut;
  });

  useEffect(() => {
    if (!capturingShortcut) return;
    const kind = capturingShortcut;
    const windows = capabilities.platform === "windows";
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        void cancelShortcutCapture();
        return;
      }
      // On Windows the helper's keyboard hook is the only source: it sees
      // which side a modifier is on and AltGr, which the DOM does not, and
      // two sources racing each other is how a capture used to end with
      // the other source's cancel wiping its result.
      if (windows) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      // Key chords are read here in the DOM (the window is focused during a
      // rebind); the helper's flagsChanged monitor only contributes fn and
      // bare-modifier chords. This split is what lets the helper run without
      // the Input Monitoring permission.
      const result = chordFromKeyEvent(event);
      if (result.kind === "ignore") return;
      event.preventDefault();
      event.stopPropagation();
      if (result.kind === "needsModifier") {
        setShortcutError(modifierRequiredMessage);
        setShortcutErrorKind(kind);
        setStatus(modifierRequiredMessage);
        return;
      }
      setShortcutError(undefined);
      void dictationHelperCommand({ type: "cancel_shortcut_capture" }).catch(() => undefined);
      void saveShortcutRef.current(kind, result.shortcut);
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [capturingShortcut, capabilities.platform]);

  async function requestMicrophones() {
    try {
      await dictationHelperCommand({ type: "list_microphones" });
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  const providerCatalogRequests = useRef<Record<ProviderModelMode, number>>({
    transcription: 0,
    generation: 0,
    image: 0,
    video: 0,
  });

  async function requestProviderModels(mode: ProviderModelMode) {
    const request = ++providerCatalogRequests.current[mode];
    try {
      const response = await listProviderModels(mode);
      if (request !== providerCatalogRequests.current[mode]) return;
      setProviderModels((models) => ({ ...models, [mode]: response.models }));
      if (mode === "generation") setGenerationCatalogFailed(false);
    } catch (error) {
      if (request !== providerCatalogRequests.current[mode]) return;
      // A stale successful fetch must not survive a failed refresh.
      setProviderModels((models) => ({ ...models, [mode]: [] }));
      if (mode === "generation") setGenerationCatalogFailed(true);
      else setStatus(messageFromError(error));
    }
  }

  function handleHelperEvent(helperEvent: DictationHelperEvent) {
    if (helperEvent.type === "microphone_devices") {
      setMicrophones(helperEvent.payload?.devices ?? []);
      setDefaultMicrophone(helperEvent.payload?.defaultDevice);
      return;
    }
    if (helperEvent.type === "mic_test_started") {
      setMicTestState("recording");
      setMicTestError(undefined);
      setMicTestSampleSrc(undefined);
      setMicTestLevel(0);
      setMicTestStartedAt(Date.now());
      setMicTestPlaying(false);
      return;
    }
    if (helperEvent.type === "mic_test_level") {
      setMicTestLevel(numericPayload(helperEvent.payload?.level));
      return;
    }
    if (helperEvent.type === "mic_test_ready") {
      const path = stringPayload(helperEvent.payload?.path);
      if (!path) {
        setMicTestState("error");
        setMicTestError(translate("settings.micTest.noSample"));
        return;
      }
      setMicTestState("ready");
      setMicTestStartedAt(undefined);
      setMicTestLevel(numericPayload(helperEvent.payload?.observedAudioLevel));
      setMicTestError(undefined);
      setMicTestPlaying(false);
      setMicTestSampleSrc(localAudioFileSrc(path));
      return;
    }
    if (helperEvent.type === "mic_test_error") {
      const message = messageForErrorCode(
        stringPayload(helperEvent.payload?.code),
        helperEvent.payload?.message ?? translate("settings.micTest.couldNotRecord"),
      );
      setMicTestState("error");
      setMicTestStartedAt(undefined);
      setMicTestError(message);
      setMicTestPlaying(false);
      setStatus(message);
      return;
    }
    if (helperEvent.type === "fn_monitor_unavailable") {
      setStatus(
        messageForErrorCode(
          stringPayload(helperEvent.payload?.code),
          helperEvent.payload?.message ?? translate("settings.shortcuts.monitorUnavailable"),
          { shortcut: stringPayload(helperEvent.payload?.shortcut) },
        ),
      );
      return;
    }
    if (helperEvent.type === "helper_unavailable") {
      setHelperUnavailable({
        reason: stringPayload(helperEvent.payload?.reason) ?? "restarting",
        message: helperEvent.payload?.message ?? translate("settings.helper.restarting"),
      });
      return;
    }
    if (helperEvent.type === "hotkey_trigger_ready") {
      // The helper re-armed the hotkey, so it is back: clear any down notice.
      setHelperUnavailable(undefined);
      return;
    }
    if (helperEvent.type === "hotkey_trigger_unavailable") {
      const message = windowsShortcutMessage(
        stringPayload(helperEvent.payload?.code),
        helperEvent.payload?.message ?? translate("settings.shortcuts.unavailable"),
        stringPayload(helperEvent.payload?.shortcut),
      );
      const kind = shortcutKindPayload(helperEvent.payload?.kind);
      setHelperUnavailable(undefined);
      setShortcutError(message);
      setShortcutErrorKind(kind);
      setStatus(message);
      return;
    }
    if (helperEvent.type === "shortcut_capture_started") {
      setStatus(translate("settings.shortcuts.pressToRecord"));
      return;
    }
    if (helperEvent.type === "shortcut_capture_error") {
      const message = windowsShortcutMessage(
        stringPayload(helperEvent.payload?.code),
        helperEvent.payload?.message ?? translate("settings.shortcuts.captureFailed"),
      );
      const kind = shortcutKindPayload(helperEvent.payload?.kind) ?? capturingShortcutRef.current;
      setCapturingShortcut(undefined);
      setShortcutError(message);
      setShortcutErrorKind(kind);
      setStatus(message);
      return;
    }
    if (helperEvent.type === "shortcut_capture_cancelled") {
      // The capture ending is not the user's next action: an error already
      // on the row (a registration the system refused) stays visible.
      setCapturingShortcut(undefined);
      setStatus(translate("settings.shortcuts.captureEnded"));
      return;
    }
    if (helperEvent.type === "shortcut_captured") {
      const kind = capturingShortcutRef.current;
      if (!kind) {
        setShortcutError(translate("settings.shortcuts.noTarget"));
        setStatus(translate("settings.shortcuts.noTarget"));
        return;
      }
      const shortcut = shortcutFromCapturePayload(helperEvent.payload?.shortcut, 1);
      if (!shortcut) {
        setShortcutError(translate("settings.shortcuts.invalidData"));
        setStatus(translate("settings.shortcuts.invalidData"));
        return;
      }
      setShortcutError(undefined);
      setShortcutErrorKind(undefined);
      void saveShortcut(kind, shortcut);
      return;
    }
    if (helperEvent.type === "error") {
      setStatus(
        messageForErrorCode(
          stringPayload(helperEvent.payload?.code),
          helperEvent.payload?.message ?? translate("settings.helper.failed"),
        ),
      );
    }
  }

  async function selectMicrophone(id?: string, name?: string) {
    try {
      if (micTestState !== "idle") {
        await resetMicTestState(true);
      }
      const next = await setDictationMicrophone(id, name);
      setSettings(next);
      setMicOpen(false);
      setStatus(
        name
          ? translate("settings.audio.microphoneSet", { name })
          : translate("settings.audio.microphoneAuto"),
      );
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function saveShortcut(
    kind: DictationShortcutKind,
    shortcut: Pick<DictationShortcutSetting, "code" | "modifiers" | "label" | "pressCount">,
  ) {
    try {
      const next = await setDictationShortcut(kind, shortcut);
      setSettings(next);
      setCapturingShortcut(undefined);
      setShortcutError(undefined);
      setShortcutErrorKind(undefined);
      setStatus(
        translate("settings.shortcuts.set", {
          shortcut: shortcutKindLabel(kind),
          label: shortcutForKind(next, kind).label,
        }),
      );
    } catch (error) {
      setShortcutError(messageFromError(error));
      setShortcutErrorKind(kind);
      setStatus(messageFromError(error));
    }
  }

  async function startShortcutCapture(kind: DictationShortcutKind) {
    setShortcutError(undefined);
    setShortcutErrorKind(undefined);
    setCapturingShortcut(kind);
    try {
      await dictationHelperCommand({
        type: "start_shortcut_capture",
        kind,
        pressCount: 1,
      });
    } catch (error) {
      setCapturingShortcut(undefined);
      setShortcutError(messageFromError(error));
      setShortcutErrorKind(kind);
      setStatus(messageFromError(error));
    }
  }

  async function cancelShortcutCapture() {
    setCapturingShortcut(undefined);
    setShortcutError(undefined);
    setShortcutErrorKind(undefined);
    try {
      await dictationHelperCommand({ type: "cancel_shortcut_capture" });
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function startMicTest() {
    setMicTestState("recording");
    setMicTestError(undefined);
    setMicTestSampleSrc(undefined);
    setMicTestLevel(0);
    setMicTestStartedAt(Date.now());
    setMicTestPlaying(false);
    try {
      await dictationHelperCommand({
        type: "start_mic_test",
        durationSeconds: MIC_TEST_DURATION_SECONDS,
      });
    } catch (error) {
      const message = messageFromError(error);
      setMicTestState("error");
      setMicTestStartedAt(undefined);
      setMicTestError(message);
      setStatus(message);
    }
  }

  async function startOverMicTest() {
    await resetMicTestState(true);
    await startMicTest();
  }

  async function resetMicTestState(discardHelper = false) {
    setMicTestState("idle");
    setMicTestLevel(0);
    setMicTestStartedAt(undefined);
    setMicTestSampleSrc(undefined);
    setMicTestError(undefined);
    setMicTestPlaying(false);
    if (!discardHelper) return;
    try {
      await dictationHelperCommand({ type: "discard_mic_test" });
    } catch {
      // Resetting the settings UI should not surface stale helper cleanup errors.
    }
  }

  // Persists the model without changing any engine selection.
  async function selectProviderModel(mode: ProviderModelMode, modelId: string): Promise<boolean> {
    if (showingPartitionModels) return false;
    try {
      const next = await setProviderModel(mode, modelId);
      setProviderSettings(next);
      setEffectiveProviderSettings(next);
      dispatchProviderModelSettingsChanged({ mode, modelId });
      setStatus(translate(`settings.models.updated.${mode}`));
      return true;
    } catch (error) {
      setStatus(messageFromError(error));
      return false;
    }
  }

  function closeModelPicker() {
    setPickerMode(undefined);
    setModelPickerFlyout(null);
    setModelSearch("");
  }

  function selectModelFromPicker(mode: ProviderModelMode, modelId: string) {
    if (!showingPartitionModels) void selectProviderModel(mode, modelId);
    closeModelPicker();
  }

  async function saveProviderApiKey(provider: BuiltinProvider) {
    const apiKey = (providerApiKeyDrafts[provider] || "").trim();
    if (!apiKey) {
      setStatus(translate("settings.providerKeys.enterKey"));
      return;
    }
    try {
      const next = await setProviderApiKey(provider, apiKey);
      setProviderSettings(next);
      setEffectiveProviderSettings((current) => ({
        ...current,
        providerApiKeysConfigured: next.providerApiKeysConfigured,
      }));
      if (provider === "openrouter") void requestProviderModels("generation");
      void requestProviderModels("image");
      void requestProviderModels("video");
      setProviderApiKeyDrafts((drafts) => ({ ...drafts, [provider]: "" }));
      setProviderApiKeyRejectedErrors((errors) => ({ ...errors, [provider]: undefined }));
      const providerName = translate(`settings.providerKeys.${provider}.name`);
      setStatus(translate("settings.providerKeys.saved", { provider: providerName }));
      dispatchProviderModelSettingsChanged({ mode: "generation", modelId: next.generationModel });
    } catch (error) {
      const code = errorCode(error);
      const message = messageFromError(error);
      if (code === "provider_api_key_rejected") {
        setProviderApiKeyRejectedErrors((errors) => ({ ...errors, [provider]: message }));
      }
      setStatus(message);
    }
  }

  async function removeProviderApiKey(provider: BuiltinProvider) {
    try {
      const next = await clearProviderApiKey(provider);
      setProviderSettings(next);
      setEffectiveProviderSettings((current) => ({
        ...current,
        providerApiKeysConfigured: next.providerApiKeysConfigured,
      }));
      if (provider === "openrouter") void requestProviderModels("generation");
      void requestProviderModels("image");
      void requestProviderModels("video");
      setProviderApiKeyDrafts((drafts) => ({ ...drafts, [provider]: "" }));
      setProviderApiKeyRejectedErrors((errors) => ({ ...errors, [provider]: undefined }));
      const providerName = translate(`settings.providerKeys.${provider}.name`);
      setStatus(translate("settings.providerKeys.removed", { provider: providerName }));
      dispatchProviderModelSettingsChanged({ mode: "generation", modelId: next.generationModel });
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function toggleLiveTranscription(enabled: boolean) {
    try {
      const next = await setLiveTranscription(enabled);
      setProviderSettings(next);
      setStatus(
        enabled
          ? translate("settings.liveTranscription.on")
          : translate("settings.liveTranscription.off"),
      );
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function toggleImageSafeMode(enabled: boolean) {
    try {
      const next = await setImageSafeMode(enabled);
      setProviderSettings(next);
      setStatus(enabled ? translate("settings.safeMode.on") : translate("settings.safeMode.off"));
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  async function selectLanguage(language: string) {
    try {
      const next = await setDictationLanguage(language || undefined);
      setSettings(next);
      setLanguageOpen(false);
      setStatus(
        language
          ? translate("settings.dictation.languageSet", { language: languageLabel(language) })
          : translate("settings.dictation.languageAuto"),
      );
    } catch (error) {
      setStatus(messageFromError(error));
    }
  }

  const microphoneName = settings.microphone.name ?? t("settings.audio.autoDetect");
  const microphoneDescription = settings.microphone.id
    ? t("settings.audio.microphoneDescription")
    : defaultMicrophone?.name
      ? t("settings.audio.autoDetectUses", { name: defaultMicrophone.name })
      : t("settings.audio.autoDetectSystem");
  const microphoneOptions = [
    { id: undefined, name: t("settings.audio.autoDetect") },
    ...microphones,
  ];
  const selectedMicrophoneIndex = Math.max(
    0,
    microphoneOptions.findIndex((option) => (option.id ?? "") === (settings.microphone.id ?? "")),
  );
  const selectedLanguageIndex = Math.max(
    0,
    LANGUAGE_OPTIONS.findIndex((option) => option.value === (settings.language ?? "")),
  );
  const displayProviderSettings = showingPartitionModels
    ? effectiveProviderSettings
    : providerSettings;
  const transcriptionOptions = modelOptions(
    providerModels.transcription.filter(
      (model) => model.provider === displayProviderSettings.transcriptionProvider,
    ),
    displayProviderSettings.transcriptionModel,
  );
  const generationOptions = modelOptions(
    providerModels.generation,
    modelValueForMode("generation"),
  );
  const notesProvider = notesProviders?.usage.notes;
  const cliNames: Record<string, string> = {
    claude: "Claude Code",
    codex: "Codex",
    pi: "Pi",
    agy: "Antigravity",
    "cursor-agent": "Cursor Agent",
    copilot: "GitHub Copilot",
  };
  const notesEngineName =
    notesProvider?.kind === "cli"
      ? (cliNames[notesProvider.id] ?? notesProvider.id)
      : notesProvider?.kind === "endpoint"
        ? (notesProviders?.endpoints.find((endpoint) => endpoint.id === notesProvider.id)?.name ??
          notesProvider.id)
        : undefined;
  const textModelDescription = notesEngineName
    ? t("settings.models.textOtherEngineDescription", { engine: notesEngineName })
    : t("settings.models.textDescription");
  const imageOptions = IMAGE_GENERATION_ENABLED
    ? modelOptions(imageModelCatalog(), displayProviderSettings.imageModel)
    : [];
  const videoOptions = VIDEO_GENERATION_ENABLED
    ? modelOptions(videoModelCatalog(), displayProviderSettings.videoModel)
    : [];

  useEffect(() => {
    if (showingPartitionModels) closeModelPicker();
  }, [showingPartitionModels]);

  useEffect(() => {
    if (!pickerMode) return;
    function onPointer(event: MouseEvent) {
      const target = event.target as Node;
      if (modelPickerPopoverRef.current?.contains(target)) return;
      if (modelPickerTriggerRef.current?.contains(target)) return;
      // The hover detail cards are portaled to document.body, so a click inside
      // one (its "Show more" toggle) lands outside the popover — treat it as in.
      if (target instanceof Element && target.closest(".agent-composer-model-hovercard")) return;
      closeModelPicker();
    }
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      if (modelPickerFlyout?.kind === "all") {
        setModelPickerFlyout(null);
        setModelSearch("");
      } else {
        closeModelPicker();
      }
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [pickerMode, modelPickerFlyout]);

  useEffect(() => {
    if (pickerMode === "image" || modelPickerFlyout?.kind === "all") {
      modelPickerSearchRef.current?.focus();
    }
  }, [pickerMode, modelPickerFlyout]);

  useEffect(() => {
    if (micOpen) updateMicrophonePopoverPlacement();
  }, [micOpen, microphoneOptions.length, selectedMicrophoneIndex]);

  useEffect(() => {
    if (languageOpen) updateLanguagePopoverPlacement();
  }, [languageOpen, selectedLanguageIndex]);

  function updateMicrophonePopoverPlacement() {
    setMicPopoverPlacement(
      selectPopoverPlacement(micWrapRef.current, microphoneOptions.length, selectedMicrophoneIndex),
    );
  }

  function updateLanguagePopoverPlacement() {
    setLanguagePopoverPlacement(
      selectPopoverPlacement(
        languageWrapRef.current,
        LANGUAGE_OPTIONS.length,
        selectedLanguageIndex,
      ),
    );
  }

  function modelOptionsForMode(mode: ProviderModelMode) {
    if (mode === "transcription") return transcriptionOptions;
    if (mode === "image") return IMAGE_GENERATION_ENABLED ? imageOptions : [];
    if (mode === "video") return VIDEO_GENERATION_ENABLED ? videoOptions : [];
    return generationOptions;
  }

  function modelValueForMode(mode: ProviderModelMode) {
    if (mode === "transcription") return displayProviderSettings.transcriptionModel;
    if (mode === "image") return displayProviderSettings.imageModel;
    if (mode === "video") return displayProviderSettings.videoModel;
    if (showingPartitionModels) {
      return partitionGenerationModel ?? globalGenerationModelValue();
    }
    return globalGenerationModelValue();
  }

  function globalGenerationModelValue() {
    return providerSettings.generationModel;
  }

  function openModelPicker(mode: ProviderModelMode) {
    if (showingPartitionModels) return;
    if (mode === "image" && !IMAGE_GENERATION_ENABLED) return;
    if (mode === "video" && !VIDEO_GENERATION_ENABLED) return;
    setPickerMode(mode);
    setModelPickerFlyout(null);
    setModelSearch("");
    // Image and video models are curated local lists, not fetched catalogs.
    void requestProviderModels(mode);
  }

  function microphonePopoverStyle(): CSSProperties {
    return selectPopoverStyle(micPopoverPlacement, selectedMicrophoneIndex);
  }

  function languagePopoverStyle(): CSSProperties {
    return selectPopoverStyle(languagePopoverPlacement, selectedLanguageIndex);
  }

  function handleReleaseVersionClick() {
    if (
      !experimentalFlags.loaded ||
      experimentalFlags.unlocked ||
      experimentalUnlockingRef.current
    ) {
      return;
    }
    const next = registerExperimentalUnlockClick(experimentalUnlockClicksRef.current, Date.now());
    experimentalUnlockClicksRef.current = next.state;
    if (!next.unlocked) return;

    experimentalUnlockingRef.current = true;
    setExperimentalError(undefined);
    void setExperimentalFlags({
      unlocked: true,
      browser_use: experimentalFlags.browser_use,
    })
      .then(() => toast(translate("settings.experiments.unlocked")))
      .catch((error) => setExperimentalError(messageFromError(error)))
      .finally(() => {
        experimentalUnlockingRef.current = false;
      });
  }

  async function updateExperimentalFlags(update: { unlocked?: boolean; browser_use?: boolean }) {
    setExperimentalOperation("flags");
    setExperimentalError(undefined);
    try {
      await setExperimentalFlags({
        unlocked: update.unlocked ?? experimentalFlags.unlocked,
        browser_use: update.browser_use ?? experimentalFlags.browser_use,
      });
    } catch (error) {
      setExperimentalError(messageFromError(error));
    } finally {
      setExperimentalOperation(undefined);
    }
  }

  return (
    <div className="settings-page" data-controlled={controlled || undefined}>
      {controlled ? null : (
        <>
          <header className="settings-header">
            <h1 className="settings-title">{t("common.settings")}</h1>
            <p className="settings-description">{t("settings.page.description")}</p>
          </header>

          <nav className="settings-nav" role="tablist" aria-label={t("settings.page.navAria")}>
            {settingsTabs.map((tab) => (
              <button
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={activeTab === tab.id}
                aria-controls={`settings-panel-${tab.id}`}
                id={`settings-tab-${tab.id}`}
                onClick={() => setActiveTab(tab.id)}
              >
                {tab.label}
              </button>
            ))}
          </nav>
        </>
      )}

      <div
        className="settings-tab-panel"
        role="tabpanel"
        id={`settings-panel-${activeTab}`}
        aria-labelledby={`settings-tab-${activeTab}`}
      >
        {activeTab === "general" ? (
          <>
            <SettingsPageHeader
              title={t("settings.tabs.general")}
              blurb={t("settings.general.blurb")}
            />
            <PermissionsSettingsSection
              microphonePermissionStatus={microphonePermissionStatus}
              microphoneReadiness={microphoneReadiness}
              accessibilityPermissionStatus={accessibilityPermissionStatus}
              systemReadiness={systemReadiness}
              onEnableMicrophone={onEnableMicrophone}
              onEnableAccessibility={onEnableAccessibility}
              onEnableSystemAudio={onEnableSystemAudio}
            />

            <StartupSettingsSection />

            {onCheckForUpdates ? (
              <UpdatesSettingsSection onCheck={onCheckForUpdates} checking={checkingForUpdates} />
            ) : null}
          </>
        ) : null}

        {activeTab === "appearance" ? (
          <section className="settings-group" aria-labelledby="appearance-heading">
            <SettingsPageHeader
              id="appearance-heading"
              title={t("settings.appearance.title")}
              blurb={t("settings.appearance.blurb")}
            />
            <div className="settings-card">
              <div className="settings-rows">
                <div className="settings-row">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title">{t("settings.interfaceLanguage.title")}</h3>
                    <p className="settings-row-description">
                      {t("settings.interfaceLanguage.description")}
                    </p>
                  </div>
                  <div className="settings-row-control">
                    <Select
                      value={interfaceLocale}
                      options={INTERFACE_LOCALE_OPTIONS.map((option) => ({
                        value: option.value,
                        label: option.label,
                        lang: option.value,
                      }))}
                      placeholder="English"
                      ariaLabel={t("settings.interfaceLanguage.aria", {
                        language:
                          INTERFACE_LOCALE_OPTIONS.find(
                            (option) => option.value === interfaceLocale,
                          )?.label ?? "English",
                      })}
                      onChange={(value) => setInterfaceLocale(value as InterfaceLocale)}
                    />
                  </div>
                </div>
                <div className="settings-row">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title">{t("settings.theme.title")}</h3>
                    <p className="settings-row-description">{t("settings.theme.description")}</p>
                  </div>
                  <div className="settings-row-control">
                    <SegmentedControl<ThemePreference>
                      aria-label={t("settings.theme.aria")}
                      value={theme}
                      options={themeOptions(t)}
                      onValueChange={(next) => {
                        setTheme(next);
                        setStoredTheme(next);
                      }}
                    />
                  </div>
                </div>
                <div className="settings-row">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title">{t("settings.textSize.title")}</h3>
                    <p className="settings-row-description">{t("settings.textSize.description")}</p>
                  </div>
                  <div className="settings-row-control">
                    <SegmentedControl<FontScaleId>
                      aria-label={t("settings.textSize.title")}
                      value={fontScale}
                      options={fontScaleOptions(t)}
                      onValueChange={setStoredFontScale}
                    />
                  </div>
                </div>
                <div className="settings-row">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title">{t("settings.accent.title")}</h3>
                    <p className="settings-row-description">{t("settings.accent.description")}</p>
                  </div>
                  <div className="settings-row-control">
                    <Select
                      className="accent-select"
                      popoverWidth="trigger"
                      value={brand}
                      options={BRAND_PRESETS.map((preset) => ({
                        value: preset.id,
                        label: preset.label,
                        color: preset.value,
                      }))}
                      placeholder="Clay"
                      ariaLabel={t("settings.accent.aria", {
                        color:
                          BRAND_PRESETS.find((preset) => preset.id === brand)?.label ??
                          BRAND_PRESETS[0].label,
                      })}
                      onChange={(id) => {
                        setBrand(id as BrandId);
                        setStoredBrand(id as BrandId);
                      }}
                    />
                  </div>
                </div>
              </div>
            </div>
          </section>
        ) : null}

        {activeTab === "shortcuts" ? (
          <section className="settings-group" aria-labelledby="shortcuts-heading">
            <SettingsPageHeader
              id="shortcuts-heading"
              title={t("settings.tabs.shortcuts")}
              blurb={t("settings.shortcuts.blurb")}
            />
            {helperUnavailable ? (
              <InlineNotice
                role="alert"
                aria-label={t("settings.helper.unavailableAria")}
                eyebrow={t("settings.helper.pausedEyebrow")}
                body={helperUnavailable.message}
              />
            ) : null}
            <div className="settings-card">
              <div className="settings-rows">
                {capabilities.shortcuts ? (
                  <>
                    <ShortcutRow
                      title={t("settings.shortcuts.pushToTalk")}
                      description={t("settings.shortcuts.pushToTalkDescription")}
                      shortcut={settings.pushToTalkShortcut}
                      defaultShortcut={defaultShortcuts.push_to_talk}
                      capturing={capturingShortcut === "push_to_talk"}
                      disabled={!!capturingShortcut && capturingShortcut !== "push_to_talk"}
                      error={shortcutErrorKind === "push_to_talk" ? shortcutError : undefined}
                      onChange={() => void startShortcutCapture("push_to_talk")}
                      onReset={() =>
                        void saveShortcut("push_to_talk", defaultShortcuts.push_to_talk)
                      }
                      onCancel={() => void cancelShortcutCapture()}
                      platform={capabilities.platform}
                      captureHintWindows={t("settings.shortcuts.captureHintWindows")}
                    />

                    <ShortcutRow
                      title={t("settings.shortcuts.toggle")}
                      description={t("settings.shortcuts.toggleDescription")}
                      shortcut={settings.toggleShortcut}
                      defaultShortcut={defaultShortcuts.toggle}
                      capturing={capturingShortcut === "toggle"}
                      disabled={!!capturingShortcut && capturingShortcut !== "toggle"}
                      error={shortcutErrorKind === "toggle" ? shortcutError : undefined}
                      onChange={() => void startShortcutCapture("toggle")}
                      onReset={() => void saveShortcut("toggle", defaultShortcuts.toggle)}
                      onCancel={() => void cancelShortcutCapture()}
                      platform={capabilities.platform}
                      captureHintWindows={t("settings.shortcuts.captureHintWindowsToggle")}
                    />
                  </>
                ) : (
                  <div className="settings-row">
                    <div className="settings-row-info">
                      <h3 className="settings-row-title">
                        {t("settings.shortcuts.unavailableTitle")}
                      </h3>
                      <p className="settings-row-description">
                        {t("settings.shortcuts.unavailableDescription")}
                      </p>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </section>
        ) : null}
        {activeTab === "activity" ? <ActivitySettingsSection /> : null}
        {activeTab === "dictation" ? (
          <>
            <section className="settings-group" aria-labelledby="dictation-heading">
              <SettingsPageHeader
                id="dictation-heading"
                title={t("settings.tabs.dictation")}
                blurb={t("settings.dictation.blurb")}
              />
              <div className="settings-card">
                <div className="settings-rows">
                  <div className="settings-row">
                    <div className="settings-row-info">
                      <h3 className="settings-row-title">{t("common.language")}</h3>
                      <p className="settings-row-description">
                        {t("settings.dictation.languageDescription")}
                      </p>
                    </div>
                    <div className="settings-row-control" ref={languageWrapRef}>
                      <button
                        type="button"
                        className="select-trigger settings-language-select"
                        aria-label={t("settings.dictation.languageAria")}
                        aria-haspopup="listbox"
                        aria-expanded={languageOpen}
                        onClick={() => setLanguageOpen((value) => !value)}
                      >
                        <span>{languageLabel(settings.language ?? "")}</span>
                        <IconChevronDownSmall size={14} />
                      </button>
                      {languageOpen ? (
                        <ul
                          className="select-popover"
                          role="listbox"
                          data-placement={languagePopoverPlacement}
                          style={languagePopoverStyle()}
                        >
                          {LANGUAGE_OPTIONS.map((option) => {
                            const selected = option.value === (settings.language ?? "");
                            return (
                              <li key={option.value || "auto"}>
                                <button
                                  type="button"
                                  role="option"
                                  aria-selected={selected}
                                  data-selected={selected}
                                  onClick={() => void selectLanguage(option.value)}
                                >
                                  <span>{option.label}</span>
                                  <span className="select-check" aria-hidden>
                                    {selected ? <IconCheckmark2Small size={14} /> : null}
                                  </span>
                                </button>
                              </li>
                            );
                          })}
                        </ul>
                      ) : null}
                    </div>
                  </div>
                </div>
              </div>
            </section>

            <StyleSettingsSection />

            <DictionarySettingsSection />
          </>
        ) : null}

        {activeTab === "audio" ? (
          <section className="settings-group" aria-labelledby="audio-heading">
            <SettingsPageHeader
              id="audio-heading"
              title={t("settings.tabs.audio")}
              blurb={
                capabilities.platform === "windows"
                  ? t("settings.audio.blurbWindows")
                  : t("settings.audio.blurb")
              }
            />
            <div className="settings-card">
              <div className="settings-rows">
                <div className="settings-row">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title">{t("settings.audio.microphone")}</h3>
                    <p className="settings-row-description">{microphoneDescription}</p>
                  </div>
                  <div className="settings-row-control" ref={micWrapRef}>
                    <button
                      type="button"
                      className="select-trigger"
                      aria-haspopup="listbox"
                      aria-expanded={micOpen}
                      onClick={() => {
                        setMicOpen((value) => !value);
                        void requestMicrophones();
                      }}
                    >
                      <span>{microphoneName}</span>
                      <IconChevronDownSmall size={14} />
                    </button>
                    {micOpen ? (
                      // selectPopoverStyle offsets for the popover chrome and
                      // the trigger/row height difference, so the selected
                      // item overlays the trigger exactly with no visual jump.
                      <ul
                        className="select-popover"
                        role="listbox"
                        data-placement={micPopoverPlacement}
                        style={microphonePopoverStyle()}
                      >
                        {microphoneOptions.map((option) => {
                          const selected = (option.id ?? "") === (settings.microphone.id ?? "");
                          return (
                            <li key={option.id ?? "auto"}>
                              <button
                                type="button"
                                role="option"
                                aria-selected={selected}
                                data-selected={selected}
                                onClick={() =>
                                  void selectMicrophone(
                                    option.id,
                                    option.id ? option.name : undefined,
                                  )
                                }
                              >
                                <span>{option.name}</span>
                                <span className="select-check" aria-hidden>
                                  {selected ? <IconCheckmark2Small size={14} /> : null}
                                </span>
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    ) : null}
                  </div>
                </div>

                {capabilities.platform === "macos" || capabilities.platform === "windows" ? (
                  <MicTestControl
                    state={micTestState}
                    level={micTestLevel}
                    startedAt={micTestStartedAt}
                    sampleSrc={micTestSampleSrc}
                    error={micTestError}
                    playing={micTestPlaying}
                    durationSeconds={MIC_TEST_DURATION_SECONDS}
                    onStart={() => void startMicTest()}
                    onStartOver={() => void startOverMicTest()}
                    onPlaybackError={() => {
                      setMicTestError(t("settings.micTest.playbackUnavailable"));
                    }}
                    onPlayingChange={setMicTestPlaying}
                  />
                ) : null}

                {systemUnavailable ? null : (
                  <div className="settings-row">
                    <div className="settings-row-info">
                      <h3 className="settings-row-title">{t("settings.audio.systemAudio")}</h3>
                      <p className="settings-row-description">
                        {t("settings.audio.systemAudioDescription")}
                      </p>
                    </div>
                    <div className="settings-row-control">
                      {systemDenied ? (
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={onEnableSystemAudio}
                        >
                          {t("common.enable")}
                        </button>
                      ) : null}
                      <Switch
                        checked={systemOn}
                        disabled={checkingSourceReadiness || systemLocked}
                        aria-label={t("settings.audio.systemAudioAria")}
                        onCheckedChange={(next) =>
                          onSourceModeChange(next ? "microphonePlusSystem" : "microphoneOnly")
                        }
                      />
                    </div>
                  </div>
                )}
              </div>
            </div>
          </section>
        ) : null}

        {activeTab === "models" ? (
          <>
            <SettingsPageHeader
              title={t("settings.tabs.models")}
              blurb={t("settings.models.blurb")}
            />
            <section
              className="settings-group settings-models-group"
              aria-labelledby="voice-models-heading"
            >
              <h2 id="voice-models-heading" className="settings-group-heading">
                {t("settings.models.voice")}
              </h2>
              <p className="settings-group-description">{t("settings.models.voiceDescription")}</p>
              {showingPartitionModels ? (
                <p className="settings-models-profile-note">
                  Showing models for the current data set: {currentDataPartitionLabel}. Switch to
                  the default data set to edit global models.
                </p>
              ) : null}
              <div className="settings-card settings-models-card">
                <div className="settings-rows">
                  <ModelRow
                    mode="transcription"
                    beforeDivider
                    title={t("settings.models.transcription")}
                    description={t("settings.models.transcriptionDescription")}
                    value={modelValueForMode("transcription")}
                    options={transcriptionOptions}
                    open={pickerMode === "transcription"}
                    summarySuppressed={pickerMode !== undefined}
                    flyout={modelPickerFlyout}
                    search={modelSearch}
                    triggerRef={modelPickerTriggerRef}
                    popoverRef={modelPickerPopoverRef}
                    searchRef={modelPickerSearchRef}
                    onToggle={() =>
                      pickerMode === "transcription"
                        ? closeModelPicker()
                        : openModelPicker("transcription")
                    }
                    onFlyoutChange={setModelPickerFlyout}
                    onSearchChange={setModelSearch}
                    onSelect={(modelId) => selectModelFromPicker("transcription", modelId)}
                    readOnly={
                      showingPartitionModels ||
                      displayProviderSettings.transcriptionProvider === "local"
                    }
                  />
                  <LocalTranscriptionSettings
                    supported={providerSettings.localTranscriptionSupported === true}
                    provider={displayProviderSettings.transcriptionProvider}
                    model={displayProviderSettings.transcriptionModel}
                    readOnly={showingPartitionModels}
                    onSelect={async (modelId) => {
                      closeModelPicker();
                      await selectProviderModel("transcription", modelId);
                    }}
                    onRemoved={async (result) => {
                      if (result?.switched_to_groq) {
                        // Removal already committed this choice; a failed read must not leave Local selected.
                        setProviderSettings((settings) =>
                          settings.transcriptionProvider === "local"
                            ? {
                                ...settings,
                                transcriptionProvider: "groq",
                                transcriptionModel: "whisper-large-v3-turbo",
                              }
                            : settings,
                        );
                        setEffectiveProviderSettings((settings) =>
                          settings.transcriptionProvider === "local"
                            ? {
                                ...settings,
                                transcriptionProvider: "groq",
                                transcriptionModel: "whisper-large-v3-turbo",
                              }
                            : settings,
                        );
                        dispatchProviderModelSettingsChanged({
                          mode: "transcription",
                          modelId:
                            providerSettings.transcriptionProvider === "local"
                              ? "whisper-large-v3-turbo"
                              : providerSettings.transcriptionModel,
                        });
                      }
                      const snapshot = providerModelSettingsSnapshot(await providerModelSettings());
                      setProviderSettings(snapshot.settings);
                      setEffectiveProviderSettings(snapshot.effectiveSettings);
                      if (!result?.switched_to_groq) {
                        dispatchProviderModelSettingsChanged({
                          mode: "transcription",
                          modelId: snapshot.settings.transcriptionModel,
                        });
                      }
                      await requestProviderModels("transcription");
                    }}
                  />
                  <div className="settings-row-divider" aria-hidden />
                  <button
                    type="button"
                    className="settings-more-options-trigger settings-more-options-row"
                    aria-label={t("settings.models.moreVoiceAria")}
                    aria-expanded={showMoreVoiceOptions}
                    aria-controls="voice-more-options-panel"
                    onClick={() => setShowMoreVoiceOptions((open) => !open)}
                  >
                    <span className="settings-row-info">
                      <span className="settings-row-title">{t("common.moreOptions")}</span>
                      <span className="settings-row-description">
                        {t("settings.models.moreVoiceDescription")}
                      </span>
                    </span>
                    <IconChevronDownSmall
                      className="settings-more-options-chevron"
                      size={14}
                      aria-hidden
                    />
                  </button>
                  {showMoreVoiceOptions ? (
                    <div id="voice-more-options-panel" className="settings-more-options-panel">
                      <div className="settings-row">
                        <div className="settings-row-info">
                          <h3 className="settings-row-title">
                            {t("settings.liveTranscription.title")}
                          </h3>
                          <p className="settings-row-description">
                            {t("settings.liveTranscription.description")}
                          </p>
                        </div>
                        <div className="settings-row-control">
                          <Switch
                            checked={providerSettings.liveTranscription}
                            aria-label={t("settings.liveTranscription.aria")}
                            onCheckedChange={toggleLiveTranscription}
                          />
                        </div>
                      </div>
                    </div>
                  ) : null}
                </div>
              </div>
            </section>

            <section
              className="settings-group settings-models-group"
              aria-labelledby="text-models-heading"
            >
              <h2 id="text-models-heading" className="settings-group-heading">
                {t("settings.models.text")}
              </h2>
              <p className="settings-group-description">
                {t("settings.models.textGroupDescription")}
              </p>
              {showingPartitionModels ? (
                <p className="settings-models-profile-note">
                  Showing models for the current data set: {currentDataPartitionLabel}. Switch to
                  the default data set to edit global models.
                </p>
              ) : null}
              <div className="settings-card settings-models-card">
                <div className="settings-rows">
                  <ModelRow
                    mode="generation"
                    title={t("settings.models.text")}
                    description={textModelDescription}
                    catalogError={generationCatalogFailed}
                    onRetry={() =>
                      showingPartitionModels
                        ? void requestProviderModels("generation")
                        : openModelPicker("generation")
                    }
                    value={modelValueForMode("generation")}
                    options={generationOptions}
                    open={pickerMode === "generation"}
                    summarySuppressed={pickerMode !== undefined}
                    flyout={modelPickerFlyout}
                    search={modelSearch}
                    triggerRef={modelPickerTriggerRef}
                    popoverRef={modelPickerPopoverRef}
                    searchRef={modelPickerSearchRef}
                    onToggle={() =>
                      pickerMode === "generation"
                        ? closeModelPicker()
                        : openModelPicker("generation")
                    }
                    onFlyoutChange={setModelPickerFlyout}
                    onSearchChange={setModelSearch}
                    onSelect={(modelId) => selectModelFromPicker("generation", modelId)}
                    readOnly={showingPartitionModels}
                  />
                  <div className="settings-row-divider" aria-hidden />
                  <button
                    type="button"
                    className="settings-more-options-trigger settings-more-options-row"
                    aria-label={t("settings.models.moreTextAria")}
                    aria-expanded={showMoreTextOptions}
                    aria-controls="text-more-options-panel"
                    onClick={() => setShowMoreTextOptions((open) => !open)}
                  >
                    <span className="settings-row-info">
                      <span className="settings-row-title">{t("common.moreOptions")}</span>
                      <span className="settings-row-description">
                        {t("settings.models.moreTextDescription")}
                      </span>
                    </span>
                    <IconChevronDownSmall
                      className="settings-more-options-chevron"
                      size={14}
                      aria-hidden
                    />
                  </button>
                  {showMoreTextOptions ? (
                    <div id="text-more-options-panel" className="settings-more-options-panel">
                      <div className="settings-row">
                        <div className="settings-row-info">
                          <h3 className="settings-row-title">{t("settings.providerKeys.title")}</h3>
                          <p className="settings-row-description">
                            {t("settings.providerKeys.description")}
                          </p>
                        </div>
                      </div>
                      {(["groq", "openrouter", "fal"] as const).map((provider) => (
                        <ProviderApiKeyRow
                          key={provider}
                          provider={provider}
                          configured={providerSettings.providerApiKeysConfigured[provider]}
                          rejectedError={providerApiKeyRejectedErrors[provider]}
                          value={providerApiKeyDrafts[provider]}
                          onValueChange={(val) =>
                            setProviderApiKeyDrafts((drafts) => ({ ...drafts, [provider]: val }))
                          }
                          onSave={() => void saveProviderApiKey(provider)}
                          onRemove={() => void removeProviderApiKey(provider)}
                        />
                      ))}
                    </div>
                  ) : null}
                </div>
              </div>
            </section>

            <LlmProvidersSection
              onProvidersChanged={setNotesProviders}
              onChatProviderChanged={async () => {
                try {
                  const modelResponse = await providerModelSettings();
                  const modelSnapshot = providerModelSettingsSnapshot(modelResponse);
                  setProviderSettings(modelSnapshot.settings);
                  setEffectiveProviderSettings(modelSnapshot.effectiveSettings);
                  dispatchProviderModelSettingsChanged({
                    mode: "generation",
                    modelId: modelSnapshot.settings.generationModel,
                  });
                } catch {
                  // Provider settings refresh failed
                }
              }}
            />

            {IMAGE_GENERATION_ENABLED || VIDEO_GENERATION_ENABLED ? (
              <section
                className="settings-group settings-models-group"
                aria-labelledby="media-generation-heading"
              >
                <h2 id="media-generation-heading" className="settings-group-heading">
                  {t("settings.models.imageAndVideo")}
                </h2>
                <p className="settings-group-description">
                  {t("settings.models.mediaDescription")}
                </p>
                {showingPartitionModels ? (
                  <p className="settings-models-profile-note">
                    Showing models for the current data set: {currentDataPartitionLabel}. Switch to
                    the default data set to edit global models.
                  </p>
                ) : null}
                <div className="settings-card settings-models-card">
                  <div className="settings-rows">
                    {IMAGE_GENERATION_ENABLED ? (
                      <ModelRow
                        mode="image"
                        beforeDivider={!VIDEO_GENERATION_ENABLED}
                        title={t("settings.models.image")}
                        description={t("settings.models.imageDescription")}
                        value={modelValueForMode("image")}
                        options={imageOptions}
                        open={pickerMode === "image"}
                        summarySuppressed={pickerMode !== undefined}
                        flyout={modelPickerFlyout}
                        search={modelSearch}
                        triggerRef={modelPickerTriggerRef}
                        popoverRef={modelPickerPopoverRef}
                        searchRef={modelPickerSearchRef}
                        onToggle={() =>
                          pickerMode === "image" ? closeModelPicker() : openModelPicker("image")
                        }
                        onFlyoutChange={setModelPickerFlyout}
                        onSearchChange={setModelSearch}
                        onSelect={(modelId) => selectModelFromPicker("image", modelId)}
                        readOnly={showingPartitionModels}
                      />
                    ) : null}
                    {VIDEO_GENERATION_ENABLED ? (
                      <ModelRow
                        mode="video"
                        beforeDivider
                        title={t("settings.models.video")}
                        description={t("settings.models.videoDescription")}
                        value={modelValueForMode("video")}
                        options={videoOptions}
                        open={pickerMode === "video"}
                        summarySuppressed={pickerMode !== undefined}
                        flyout={modelPickerFlyout}
                        search={modelSearch}
                        triggerRef={modelPickerTriggerRef}
                        popoverRef={modelPickerPopoverRef}
                        searchRef={modelPickerSearchRef}
                        onToggle={() =>
                          pickerMode === "video" ? closeModelPicker() : openModelPicker("video")
                        }
                        onFlyoutChange={setModelPickerFlyout}
                        onSearchChange={setModelSearch}
                        onSelect={(modelId) => selectModelFromPicker("video", modelId)}
                        readOnly={showingPartitionModels}
                      />
                    ) : null}
                    <div className="settings-row-divider" aria-hidden />
                    <button
                      type="button"
                      className="settings-more-options-trigger settings-more-options-row"
                      aria-label={t("settings.models.moreMediaAria")}
                      aria-expanded={showMoreImageOptions}
                      aria-controls="image-more-options-panel"
                      onClick={() => setShowMoreImageOptions((open) => !open)}
                    >
                      <span className="settings-row-info">
                        <span className="settings-row-title">{t("common.moreOptions")}</span>
                        <span className="settings-row-description">
                          {t("settings.models.moreMediaDescription")}
                        </span>
                      </span>
                      <IconChevronDownSmall
                        className="settings-more-options-chevron"
                        size={14}
                        aria-hidden
                      />
                    </button>
                    {showMoreImageOptions ? (
                      <div id="image-more-options-panel" className="settings-more-options-panel">
                        <div className="settings-row">
                          <div className="settings-row-info">
                            <h3 className="settings-row-title">{t("settings.safeMode.title")}</h3>
                            <p className="settings-row-description">
                              {VIDEO_GENERATION_ENABLED
                                ? t("settings.safeMode.descriptionWithVideo")
                                : t("settings.safeMode.description")}
                            </p>
                          </div>
                          <div className="settings-row-control">
                            <Switch
                              checked={providerSettings.imageSafeMode}
                              aria-label={t("settings.safeMode.aria")}
                              onCheckedChange={toggleImageSafeMode}
                            />
                          </div>
                        </div>
                      </div>
                    ) : null}
                  </div>
                </div>
              </section>
            ) : null}

            {status ? (
              <p className="settings-status" role="status">
                {status}
              </p>
            ) : null}
          </>
        ) : null}

        {activeTab === "agent" ? (
          <AgentSettingsSection folders={folders} onFoldersImported={onFoldersImported} />
        ) : null}

        {activeTab === "mcp" ? (
          <>
            <SettingsPageHeader title={t("mcpServer.tab")} />
            <McpServerSection />
          </>
        ) : null}

        {activeTab === "memory" ? (
          <MemorySettingsSection
            folders={folders}
            initialFolderFilter={memoryFolderFilter}
            onOpenProject={onOpenProject}
          />
        ) : null}

        {activeTab === "connectors" ? (
          <>
            <ConnectorsSection />
            <AgentMcpServersSection />
          </>
        ) : null}

        {activeTab === "about" ? (
          <section className="settings-group" aria-labelledby="about-heading">
            <SettingsPageHeader
              id="about-heading"
              title={t("settings.tabs.about")}
              blurb={t("settings.about.blurb")}
            />
            <div className="settings-card">
              <div className="settings-rows">
                <div className="settings-row settings-row-meta">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title settings-meta-label">
                      {t("settings.about.releaseVersion")}
                    </h3>
                  </div>
                  <div className="settings-row-control">
                    <button
                      type="button"
                      className="settings-meta-value settings-meta-unlock-trigger"
                      onClick={handleReleaseVersionClick}
                    >
                      {APP_VERSION}
                    </button>
                  </div>
                </div>

                <div className="settings-row settings-row-meta">
                  <div className="settings-row-info">
                    <h3 className="settings-row-title settings-meta-label">
                      {t("settings.about.commit")}
                    </h3>
                  </div>
                  <div className="settings-row-control">
                    <span className="settings-meta-value settings-meta-value-mono">
                      {APP_COMMIT_HASH}
                    </span>
                  </div>
                </div>

                {import.meta.env.DEV ? (
                  // Dev builds only: same helper the devtools console exposes
                  // as clovy.replayOnboarding() — clears completion and
                  // reloads into the wizard.
                  <div className="settings-row">
                    <div className="settings-row-info">
                      <h3 className="settings-row-title">{t("settings.about.replayOnboarding")}</h3>
                      <p className="settings-row-description">
                        {t("settings.about.replayOnboardingDescription")}
                      </p>
                    </div>
                    <div className="settings-row-control">
                      <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={() => replayOnboarding()}
                      >
                        {t("settings.about.replayOnboarding")}
                      </button>
                    </div>
                  </div>
                ) : null}
              </div>
            </div>

            {experimentalFlags.unlocked ? (
              <div className="settings-card experiments-card">
                <div className="settings-rows">
                  <div className="settings-row">
                    <div className="settings-row-info">
                      <h3 className="settings-row-title">{t("settings.experiments.title")}</h3>
                      <p className="settings-row-description">
                        {t("settings.experiments.description")}
                      </p>
                    </div>
                    <div className="settings-row-control">
                      <button
                        type="button"
                        className="btn btn-secondary"
                        disabled={experimentalOperation !== undefined}
                        onClick={() => void updateExperimentalFlags({ unlocked: false })}
                      >
                        {t("settings.experiments.hide")}
                      </button>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="settings-row-info">
                      <h3 className="settings-row-title">{t("settings.experiments.browserUse")}</h3>
                      <p className="settings-row-description">
                        {t("settings.experiments.browserUseDescription")}
                      </p>
                    </div>
                    <div className="settings-row-control">
                      <Switch
                        checked={experimentalFlags.browser_use}
                        disabled={experimentalOperation !== undefined}
                        aria-label={t("settings.experiments.browserUseAria")}
                        onCheckedChange={(browser_use) =>
                          void updateExperimentalFlags({ browser_use })
                        }
                      />
                    </div>
                  </div>
                </div>
                {experimentalError ? (
                  <p className="settings-row-error" role="alert">
                    {experimentalError}
                  </p>
                ) : null}
              </div>
            ) : null}
          </section>
        ) : null}
      </div>
    </div>
  );
}

type PermissionStatusTone = "allowed" | "attention" | "blocked" | "unsupported" | "unknown";

type PermissionStatusView = {
  label: string;
  tone: PermissionStatusTone;
};

/** Launch-at-login toggle. Reads and writes the OS login item directly (the
 * LaunchAgent is the single source of truth), so state here can never drift
 * from what System Settings shows. Hidden in browser previews, where no
 * autostart backend exists. */
function UpdatesSettingsSection({ onCheck, checking }: { onCheck: () => void; checking: boolean }) {
  const t = useT();
  return (
    <section className="settings-group" aria-labelledby="updates-heading">
      <h2 id="updates-heading" className="settings-group-heading">
        {t("settings.updates.title")}
      </h2>
      <p className="settings-group-description">{t("settings.updates.description")}</p>
      <div className="settings-card">
        <div className="settings-rows">
          <div className="settings-row">
            <div className="settings-row-info">
              <h3 className="settings-row-title">{t("settings.updates.installed")}</h3>
              <p className="settings-row-description">{APP_VERSION}</p>
            </div>
            <div className="settings-row-control">
              <button
                type="button"
                className="btn btn-secondary"
                disabled={checking}
                aria-busy={checking || undefined}
                onClick={onCheck}
              >
                {checking ? t("settings.updates.checking") : t("settings.updates.check")}
              </button>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

export function StartupSettingsSection() {
  const t = useT();
  const [enabled, setEnabled] = useState<boolean>();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const [hideDock, setHideDock] = useState<boolean>();
  const [dockSaving, setDockSaving] = useState(false);
  const [dockError, setDockError] = useState<string>();
  const mac = isMacLikePlatform();

  useEffect(() => {
    if (!mac) return;
    let active = true;
    appPreferencesGet()
      .then((value) => {
        if (active) setHideDock(value.hideFromDock);
      })
      .catch(() => {
        if (active) setDockError(translate("settings.startup.dockReadError"));
      });
    return () => {
      active = false;
    };
  }, [mac]);

  async function toggleDock(next: boolean) {
    setDockSaving(true);
    setDockError(undefined);
    try {
      const value = await appPreferencesSet({ hideFromDock: next });
      setHideDock(value.hideFromDock);
    } catch {
      setDockError(translate("settings.startup.dockUpdateError"));
    } finally {
      setDockSaving(false);
    }
  }

  useEffect(() => {
    if (!autostartSupported()) return;
    let cancelled = false;
    autostartEnabled()
      .then((value) => {
        if (!cancelled) setEnabled(value);
      })
      .catch(() => {
        if (!cancelled) setError(translate("settings.startup.readError"));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function toggle(next: boolean) {
    setSaving(true);
    setError(undefined);
    try {
      await setAutostartEnabled(next);
      setEnabled(next);
    } catch {
      setError(translate("settings.startup.updateError"));
    } finally {
      setSaving(false);
    }
  }

  if (!mac && (!autostartSupported() || (enabled === undefined && !error))) return null;

  return (
    <section className="settings-group" aria-labelledby="startup-heading">
      <h2 id="startup-heading" className="settings-group-heading">
        {t("settings.startup.title")}
      </h2>
      <p className="settings-group-description">{t("settings.startup.description")}</p>
      <div className="settings-card">
        <div className="settings-rows">
          {autostartSupported() ? (
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("settings.startup.openAtLogin")}</h3>
                <p className="settings-row-description">
                  {t("settings.startup.openAtLoginDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={enabled === true}
                  disabled={saving || enabled === undefined}
                  aria-label={t("settings.startup.openAtLogin")}
                  onCheckedChange={(next) => void toggle(next)}
                />
              </div>
            </div>
          ) : null}
          {error ? <p className="settings-row-description">{error}</p> : null}
          {mac ? (
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("settings.startup.hideDock")}</h3>
                <p className="settings-row-description">
                  {t("settings.startup.hideDockDescription")}
                </p>
                {dockError ? (
                  <p className="settings-row-error" role="alert">
                    {dockError}
                  </p>
                ) : null}
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={hideDock === true}
                  disabled={dockSaving || hideDock === undefined}
                  aria-label={t("settings.startup.hideDock")}
                  onCheckedChange={(next) => void toggleDock(next)}
                />
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </section>
  );
}

function PermissionsSettingsSection({
  microphonePermissionStatus,
  microphoneReadiness,
  accessibilityPermissionStatus,
  systemReadiness,
  onEnableMicrophone,
  onEnableAccessibility,
  onEnableSystemAudio,
}: {
  microphonePermissionStatus?: string;
  microphoneReadiness?: RecordingSourceReadinessDto["sources"][number];
  accessibilityPermissionStatus?: string;
  systemReadiness?: RecordingSourceReadinessDto["sources"][number];
  onEnableMicrophone?: () => void;
  onEnableAccessibility?: () => void;
  onEnableSystemAudio: () => void;
}) {
  const t = useT();
  const macLikePlatform = fallbackDictationCapabilities().platform === "macos";
  const systemAudioSupportedPlatform = isSystemAudioSupportedPlatform();
  return (
    <section className="settings-group" aria-labelledby="permissions-heading">
      <h2 id="permissions-heading" className="settings-group-heading">
        {macLikePlatform
          ? t("settings.permissions.systemTitle")
          : t("settings.permissions.audioAccessTitle")}
      </h2>
      <p className="settings-group-description">
        {macLikePlatform
          ? t("settings.permissions.macDescription")
          : systemAudioSupportedPlatform
            ? t("settings.permissions.systemAudioDescription")
            : t("settings.permissions.micOnlyDescription")}
      </p>
      <div className="settings-card">
        <div className="settings-rows">
          <PermissionRow
            title={t("settings.audio.microphone")}
            description={t("settings.permissions.microphoneDescription")}
            status={permissionStatus(
              t,
              microphonePermissionStatus ?? microphoneReadiness?.permissionState,
            )}
            onManage={onEnableMicrophone}
          />

          {macLikePlatform ? (
            <>
              <PermissionRow
                title={t("settings.permissions.accessibility")}
                description={t("settings.permissions.accessibilityDescription")}
                status={permissionStatus(t, accessibilityPermissionStatus)}
                onManage={onEnableAccessibility}
              />

              {systemAudioSupportedPlatform ? (
                <PermissionRow
                  title={t("settings.audio.systemAudio")}
                  description={t("settings.permissions.systemAudioRowDescription")}
                  status={sourcePermissionStatus(t, systemReadiness, macLikePlatform)}
                  onManage={onEnableSystemAudio}
                />
              ) : null}
            </>
          ) : systemAudioSupportedPlatform ? (
            <PermissionRow
              title={t("settings.audio.systemAudio")}
              description={t("settings.permissions.systemAudioRowDescription")}
              status={sourcePermissionStatus(t, systemReadiness, macLikePlatform)}
            />
          ) : null}
        </div>
      </div>
    </section>
  );
}

function PermissionRow({
  title,
  description,
  status,
  onManage,
  actionLabel,
  actionText,
}: {
  title: string;
  description: string;
  status: PermissionStatusView;
  onManage?: () => void;
  actionLabel?: string;
  actionText?: string;
}) {
  const t = useT();
  const actionDisabled = status.tone === "unsupported" || !onManage;
  return (
    <div className="settings-row">
      <div className="settings-row-info">
        <h3 className="settings-row-title">{title}</h3>
        <p className="settings-row-description">{description}</p>
      </div>
      <div className="settings-row-control settings-permission-control">
        <span
          className="settings-permission-status"
          data-status={status.tone}
          role="img"
          aria-label={status.label}
          title={status.label}
        >
          <PermissionStatusIcon tone={status.tone} />
        </span>
        {onManage ? (
          <button
            type="button"
            className="btn btn-secondary"
            disabled={actionDisabled}
            aria-label={actionLabel ?? t("settings.permissions.manageAria", { name: title })}
            onClick={onManage}
          >
            {actionText ?? t("settings.permissions.manage")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function PermissionStatusIcon({ tone }: { tone: PermissionStatusTone }) {
  if (tone === "allowed") return <IconCircleCheck size={16} />;
  if (tone === "unknown") return <IconCircleQuestionmark size={16} />;
  if (tone === "unsupported") return <IconCircleX size={16} />;
  return <IconExclamationCircle size={16} />;
}

function permissionStatus(t: TFunction, state?: string): PermissionStatusView {
  switch (state) {
    case "granted":
      return { label: t("settings.permissions.status.allowed"), tone: "allowed" };
    case "denied":
      return { label: t("settings.permissions.status.blocked"), tone: "blocked" };
    case "restricted":
      return { label: t("settings.permissions.status.restricted"), tone: "blocked" };
    case "missing":
      return { label: t("settings.permissions.status.needsAccess"), tone: "attention" };
    case "not_determined":
      return { label: t("settings.permissions.status.notRequested"), tone: "attention" };
    case "unavailable":
      return { label: t("settings.permissions.status.noMicrophone"), tone: "attention" };
    case "unsupported":
      return { label: t("settings.permissions.status.unsupported"), tone: "unsupported" };
    case "unknown":
      return { label: t("settings.permissions.status.unknown"), tone: "unknown" };
    default:
      return { label: t("settings.permissions.status.checking"), tone: "unknown" };
  }
}

function sourcePermissionStatus(
  t: TFunction,
  source: RecordingSourceReadinessDto["sources"][number] | undefined,
  macLikePlatform: boolean,
): PermissionStatusView {
  if (!source) return { label: t("settings.permissions.status.checking"), tone: "unknown" };
  // The two halves are independent: permissionState is the platform grant or
  // endpoint status, while `ready` says whether this device can actually
  // capture. A microphone-only check never asks for the grant/status, and a
  // granted source can still be uncapturable.
  if (source.permissionState === "granted") {
    return source.ready
      ? {
          label: macLikePlatform
            ? t("settings.permissions.status.allowed")
            : t("settings.permissions.status.available"),
          tone: "allowed",
        }
      : { label: t("settings.permissions.status.unavailable"), tone: "attention" };
  }
  return permissionStatus(t, source.permissionState);
}

function stringPayload(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

function numericPayload(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    return Math.max(0, Math.min(1, value));
  }
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return Math.max(0, Math.min(1, parsed));
  }
  return 0;
}

function ModelRow({
  mode,
  beforeDivider = false,
  title,
  description,
  value,
  options,
  open,
  flyout,
  search,
  triggerRef,
  popoverRef,
  searchRef,
  onToggle,
  onFlyoutChange,
  onSearchChange,
  onSelect,
  readOnly = false,
  summarySuppressed,
  catalogError,
  onRetry,
}: {
  mode: ProviderModelMode;
  beforeDivider?: boolean;
  title: string;
  description: string;
  value: string;
  options: ProviderModelDto[];
  open: boolean;
  flyout: ModelPickerFlyout;
  search: string;
  triggerRef: RefObject<HTMLButtonElement>;
  popoverRef: RefObject<HTMLDivElement>;
  searchRef: RefObject<HTMLInputElement>;
  onToggle: () => void;
  onFlyoutChange: (flyout: ModelPickerFlyout) => void;
  onSearchChange: (value: string) => void;
  onSelect: (modelId: string) => void;
  readOnly?: boolean;
  summarySuppressed?: boolean;
  catalogError?: boolean;
  onRetry?: () => void;
}) {
  const t = useT();
  const model = selectedModel(options, value);
  const modelLabel = t(`settings.models.modelLabel.${mode}`);
  return (
    <div
      className={`settings-row settings-model-row${
        beforeDivider ? " settings-row-before-divider" : ""
      }`}
    >
      <div className="settings-row-info">
        <h3 className="settings-row-title">{title}</h3>
        <p className="settings-row-description">{description}</p>
        {catalogError ? (
          <div role="status">
            <p className="settings-row-description">{t("settings.models.catalogFailed")}</p>
            <button type="button" className="btn btn-secondary" onClick={onRetry}>
              {t("settings.models.retryCatalog")}
            </button>
          </div>
        ) : null}
      </div>
      <div className="settings-row-control settings-model-control">
        <HoverTip
          tip={<ModelSummaryHoverDetails model={model} />}
          className="model-summary-tip-anchor"
          width={280}
          delay={280}
          suppressed={summarySuppressed || open}
          interactive
        >
          <button
            ref={open && !readOnly ? triggerRef : undefined}
            type="button"
            className="model-summary-button"
            onClick={readOnly ? undefined : onToggle}
            aria-label={t("settings.models.changeAria", { model: modelLabel })}
            aria-haspopup="dialog"
            aria-expanded={readOnly ? false : open}
            disabled={readOnly}
          >
            <span className="model-summary-logo" aria-hidden>
              <ProviderLogo provider={model.provider} id={model.id} name={model.name} />
            </span>
            <span className="model-summary-name">{model.name}</span>
            <IconChevronDownSmall size={14} aria-hidden />
          </button>
        </HoverTip>
        {open && !readOnly ? (
          <ModelPickerPopover
            mode={mode}
            flyout={flyout}
            model={model}
            options={options}
            search={search}
            popoverRef={popoverRef}
            searchRef={searchRef}
            className="settings-model-popover"
            title={mode === "generation" ? undefined : t(`settings.models.popoverTitle.${mode}`)}
            ariaLabel={t("settings.models.chooseAria", { model: modelLabel })}
            onFlyoutChange={onFlyoutChange}
            onSearchChange={onSearchChange}
            onSelect={onSelect}
          />
        ) : null}
      </div>
    </div>
  );
}

function ModelSummaryHoverDetails({ model }: { model: ProviderModelDto }) {
  return (
    <div className="agent-composer-model-detail model-summary-hovercard">
      {/* Read-only summary card: full description in one hover (the card shows
          it in a capped scroll box; there is no "Show more" toggle anywhere). */}
      <ModelPickerCardContent model={model} withDescription />
    </div>
  );
}

function ProviderApiKeyRow({
  id,
  provider,
  configured,
  rejectedError,
  value,
  onValueChange,
  onSave,
  onRemove,
}: {
  id?: string;
  provider: BuiltinProvider;
  configured: boolean;
  rejectedError?: string;
  value: string;
  onValueChange: (value: string) => void;
  onSave: () => void;
  onRemove: () => void;
}) {
  const t = useT();
  const canSave = value.trim().length > 0;
  const title = t(`settings.providerKeys.${provider}.name`);
  const description = t(`settings.providerKeys.${provider}.description`);
  return (
    <div id={id} className="settings-row settings-row-provider-key">
      <div className="settings-row-info">
        <h3 className="settings-row-title">{title}</h3>
        <p className="settings-row-description">{description}</p>
        {configured ? (
          <p className="settings-row-description settings-row-substatus">
            {t("settings.providerKeys.configured")}
          </p>
        ) : rejectedError ? (
          <p className="settings-row-description settings-row-error" role="alert">
            {rejectedError}
          </p>
        ) : (
          <p className="settings-row-description settings-row-muted">
            {t("settings.providerKeys.missing")}
          </p>
        )}
      </div>
      <div className="settings-row-control settings-secret-control">
        <label className="settings-field settings-secret-field">
          <span>{t("settings.providerKeys.apiKey")}</span>
          <input
            type="password"
            className="dialog-input"
            value={value}
            autoComplete="off"
            spellCheck={false}
            placeholder={configured ? t("settings.providerKeys.savedHidden") : title}
            aria-label={`${title} ${t("settings.providerKeys.apiKey")}`}
            onChange={(event) => onValueChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && canSave) onSave();
            }}
          />
        </label>
        <button type="button" className="btn btn-secondary" disabled={!canSave} onClick={onSave}>
          {t("common.save")}
        </button>
        {configured ? (
          <button type="button" className="btn btn-secondary" onClick={onRemove}>
            {t("common.remove")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function ShortcutRow({
  title,
  description,
  shortcut,
  defaultShortcut,
  capturing,
  disabled,
  error,
  onChange,
  onReset,
  onCancel,
  platform,
  captureHintWindows,
}: {
  title: string;
  description: string;
  shortcut: DictationShortcutSetting;
  defaultShortcut: DictationShortcutSetting;
  captureHintWindows: string;
  capturing: boolean;
  disabled: boolean;
  error?: string;
  onChange: () => void;
  onReset: () => void;
  onCancel: () => void;
  platform: "macos" | "windows" | "unsupported";
}) {
  const t = useT();
  const canReset = !capturing && !shortcutsMatch(shortcut, defaultShortcut) && !disabled;

  return (
    <div className="settings-row">
      <div className="settings-row-info">
        <h3 className="settings-row-title">{title}</h3>
        <p className="settings-row-description" aria-live="polite">
          {capturing && platform === "windows" ? captureHintWindows : description}
        </p>
        {error ? (
          <p className="settings-row-error" role="alert">
            {error}
          </p>
        ) : null}
      </div>
      <div className="settings-row-control">
        <KeycapShortcut label={shortcut.label} capturing={capturing} platform={platform} />
        <button
          type="button"
          className="btn btn-secondary"
          disabled={disabled}
          onClick={capturing ? onCancel : onChange}
        >
          {capturing ? t("common.cancel") : t("settings.shortcuts.change")}
        </button>
        {canReset ? (
          <button
            type="button"
            className="btn btn-secondary"
            aria-label={t("settings.shortcuts.resetAria", { name: title })}
            onClick={onReset}
          >
            {t("settings.shortcuts.reset")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

/** The helper's Windows capture and registration codes, in the interface
 * language; anything else keeps the helper's own message. */
function windowsShortcutMessage(code: string | undefined, fallback: string, shortcut?: string) {
  switch (code) {
    case "no_key_detected":
      return translate("settings.shortcuts.noKeyWindows");
    case "modifier_required":
      return translate("settings.shortcuts.modifierRequiredWindows");
    case "unsupported_key":
      return translate("settings.shortcuts.unsupportedKeyWindows");
  }
  if (code?.startsWith("register_hotkey_") && shortcut) {
    return translate("settings.shortcuts.registerFailedWindows", { shortcut });
  }
  return fallback;
}

function shortcutKindPayload(value: unknown): DictationShortcutKind | undefined {
  return value === "push_to_talk" || value === "toggle" ? value : undefined;
}

function shortcutKindLabel(kind: DictationShortcutKind) {
  return kind === "toggle"
    ? translate("settings.shortcuts.toggle")
    : translate("settings.shortcuts.pushToTalk");
}

function shortcutForKind(settings: DictationSettingsDto, kind: DictationShortcutKind) {
  return kind === "toggle" ? settings.toggleShortcut : settings.pushToTalkShortcut;
}

function shortcutsMatch(first: DictationShortcutSetting, second: DictationShortcutSetting) {
  const keyCodesMatch =
    first.keyCode === undefined || second.keyCode === undefined || first.keyCode === second.keyCode;

  return (
    keyCodesMatch &&
    first.code === second.code &&
    first.label === second.label &&
    first.pressCount === second.pressCount &&
    first.modifiers.command === second.modifiers.command &&
    first.modifiers.control === second.modifiers.control &&
    first.modifiers.option === second.modifiers.option &&
    first.modifiers.shift === second.modifiers.shift &&
    first.modifiers.function === second.modifiers.function
  );
}

function messageFromError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}

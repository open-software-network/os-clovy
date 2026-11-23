import { useCallback, useEffect, useRef, useState } from "react";
import { useClovyDataChanged } from "../../lib/clovy-data-events";
import type { AgentSkillDto } from "../../lib/agent-runtime-contract";
import {
  listAgentSkills,
  readAgentSkill,
  setAgentSkillEnabled,
  updateAgentSkill,
  type FolderDto,
} from "../../lib/tauri";
import {
  AGENT_SOUNDS_CHANGED_EVENT,
  getAgentSoundsEnabled,
  setAgentSoundsEnabled,
  type AgentSoundsChangedDetail,
} from "../../lib/agent-sound-settings";
import { t as translate, useT } from "../../i18n";
import { Dialog } from "../ui/Dialog";
import { Switch } from "../ui/Switch";
import { SettingsPageHeader } from "./AppSettings";

/** Settings owned by Clovy's local agent harness. Messaging compatibility,
 * raw runtime diagnostics, and the legacy filesystem browser are
 * intentionally absent. */
export function AgentSettingsSection({
  folders: _folders = [],
  onFoldersImported: _onFoldersImported,
}: {
  selectedPlatformId?: string;
  onSelectPlatform?: (platformId: string) => void;
  onBackFromPlatform?: () => void;
  folders?: FolderDto[];
  onFoldersImported?: (folders: FolderDto[]) => void;
} = {}) {
  const t = useT();
  const [soundsEnabled, setSoundsEnabledState] = useState(getAgentSoundsEnabled);
  const [skills, setSkills] = useState<AgentSkillDto[]>();
  const [savingSkillId, setSavingSkillId] = useState<string>();
  const [editingSkill, setEditingSkill] = useState<AgentSkillDto>();
  const [skillDraft, setSkillDraft] = useState("");
  const [skillSaving, setSkillSaving] = useState(false);
  const [error, setError] = useState<string>();
  const [actionError, setActionError] = useState<string>();
  const skillsReadVersion = useRef(0);
  const editReadVersion = useRef(0);
  const mounted = useRef(true);
  const refreshSkills = useCallback(async (eventIsCurrent: () => boolean = () => true) => {
    const version = ++skillsReadVersion.current;
    const isCurrent = () =>
      mounted.current && version === skillsReadVersion.current && eventIsCurrent();
    try {
      const next = await listAgentSkills();
      if (isCurrent()) {
        setSkills(next);
        setError(undefined);
      }
    } catch (cause) {
      if (isCurrent()) setError(messageFromError(cause));
    }
  }, []);
  useClovyDataChanged("skills", (_change, isCurrent) => refreshSkills(isCurrent), true);

  useEffect(() => {
    mounted.current = true;
    void refreshSkills();
    return () => {
      mounted.current = false;
      skillsReadVersion.current += 1;
    };
  }, [refreshSkills]);

  function beginSkillMutation() {
    skillsReadVersion.current += 1;
    return () => mounted.current;
  }

  function reconcileFailedSkillMutation(isCurrent: () => boolean, cause: unknown) {
    if (!isCurrent()) return;
    setActionError(messageFromError(cause));
    // Installation-wide skills still reconcile after a refused mutation. Keep
    // the editable draft and mutation error independent of catalog reads.
    void refreshSkills();
  }

  function applySkillMutation(updated: AgentSkillDto) {
    if (!mounted.current) return;
    skillsReadVersion.current += 1;
    setSkills((current) => current?.map((item) => (item.id === updated.id ? updated : item)));
    setError(undefined);
    setActionError(undefined);
    void refreshSkills();
  }

  useEffect(() => {
    const handleSounds = (event: Event) => {
      const detail = (event as CustomEvent<AgentSoundsChangedDetail>).detail;
      if (detail) setSoundsEnabledState(detail.enabled);
    };
    window.addEventListener(AGENT_SOUNDS_CHANGED_EVENT, handleSounds);
    return () => {
      window.removeEventListener(AGENT_SOUNDS_CHANGED_EVENT, handleSounds);
    };
  }, []);

  async function changeSkill(skill: AgentSkillDto, enabled: boolean) {
    setSavingSkillId(skill.id);
    const isCurrent = beginSkillMutation();
    try {
      const updated = await setAgentSkillEnabled(skill.id, enabled);
      applySkillMutation(updated);
    } catch (cause) {
      reconcileFailedSkillMutation(isCurrent, cause);
    } finally {
      if (isCurrent()) setSavingSkillId(undefined);
    }
  }

  async function editSkill(skill: AgentSkillDto) {
    const version = ++editReadVersion.current;
    const isCurrent = () => mounted.current && version === editReadVersion.current;
    try {
      const document = await readAgentSkill(skill.id);
      if (!isCurrent()) return;
      // A successful explicit read starts a new editor flow. Catalog repair
      // alone never clears the previous action's failure or draft.
      setActionError(undefined);
      setSkillDraft(document.content);
      setEditingSkill(skill);
    } catch (cause) {
      // The latest explicit action supersedes an older save/toggle failure;
      // a background catalog response must not hide this read failure either.
      if (isCurrent()) setActionError(messageFromError(cause));
    }
  }

  async function saveSkill() {
    if (!editingSkill) return;
    setSkillSaving(true);
    const isCurrent = beginSkillMutation();
    try {
      const updated = await updateAgentSkill(editingSkill.id, skillDraft);
      if (isCurrent()) {
        applySkillMutation(updated);
        setEditingSkill(undefined);
      }
    } catch (cause) {
      reconcileFailedSkillMutation(isCurrent, cause);
    } finally {
      if (isCurrent()) setSkillSaving(false);
    }
  }

  return (
    <>
      <section className="settings-group" aria-labelledby="agent-heading">
        <SettingsPageHeader
          id="agent-heading"
          title={t("settingsPanels.agent.title")}
          blurb={t("settingsPanels.agent.blurb")}
        />
      </section>

      <section className="settings-group" aria-labelledby="agent-experience-heading">
        <h2 id="agent-experience-heading" className="settings-group-heading">
          {t("settingsPanels.agent.experience")}
        </h2>
        <p className="settings-group-description">
          {t("settingsPanels.agent.experienceDescription")}
        </p>
        <div className="settings-card">
          <div className="settings-rows">
            <div className="settings-row">
              <div className="settings-row-info">
                <h3 className="settings-row-title">{t("settingsPanels.agent.sounds")}</h3>
                <p className="settings-row-description">
                  {t("settingsPanels.agent.soundsDescription")}
                </p>
              </div>
              <div className="settings-row-control">
                <Switch
                  checked={soundsEnabled}
                  onCheckedChange={(enabled) => {
                    setSoundsEnabledState(enabled);
                    setAgentSoundsEnabled(enabled);
                  }}
                  aria-label={t("settingsPanels.agent.soundsAria")}
                />
              </div>
            </div>
          </div>
        </div>
      </section>

      <section className="settings-group" aria-labelledby="agent-skills-heading">
        <h2 id="agent-skills-heading" className="settings-group-heading">
          {t("settingsPanels.agent.skills")}
        </h2>
        <p className="settings-group-description">{t("settingsPanels.agent.skillsDescription")}</p>
        <div className="settings-card">
          <div className="settings-rows">
            {skills?.map((skill) => (
              <div className="settings-row" key={skill.id}>
                <div className="settings-row-info">
                  <h3 className="settings-row-title">{skill.name}</h3>
                  <p className="settings-row-description">{skill.description}</p>
                </div>
                <div className="settings-row-control">
                  {skill.editable ? (
                    <button
                      type="button"
                      className="btn btn-ghost"
                      onClick={() => void editSkill(skill)}
                    >
                      {t("common.edit")}
                    </button>
                  ) : null}
                  <Switch
                    checked={skill.configuredEnabled}
                    disabled={savingSkillId === skill.id || !skill.editable}
                    onCheckedChange={(enabled) => void changeSkill(skill, enabled)}
                    aria-label={
                      skill.configuredEnabled
                        ? t("settingsPanels.agent.disableSkill", { name: skill.name })
                        : t("settingsPanels.agent.enableSkill", { name: skill.name })
                    }
                  />
                </div>
              </div>
            ))}
            {skills?.length === 0 ? (
              <p className="settings-empty">{t("settingsPanels.agent.noSkills")}</p>
            ) : null}
          </div>
        </div>
        {actionError || error ? (
          <p className="settings-row-error" role="alert">
            {actionError ?? error}
          </p>
        ) : null}
      </section>
      <Dialog
        open={Boolean(editingSkill)}
        onClose={() => setEditingSkill(undefined)}
        title={
          editingSkill
            ? t("settingsPanels.agent.editSkillName", { name: editingSkill.name })
            : t("settingsPanels.agent.editSkill")
        }
        description={t("settingsPanels.agent.editSkillDescription")}
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              onClick={() => setEditingSkill(undefined)}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              disabled={skillSaving}
              onClick={() => void saveSkill()}
            >
              {skillSaving ? t("common.saving") : t("settingsPanels.agent.saveSkill")}
            </button>
          </>
        }
      >
        <textarea
          className="settings-skill-editor"
          aria-label={t("settingsPanels.agent.skillInstructions")}
          value={skillDraft}
          onChange={(event) => setSkillDraft(event.currentTarget.value)}
          rows={18}
          spellCheck={false}
        />
      </Dialog>
    </>
  );
}

function messageFromError(error: unknown) {
  if (error && typeof error === "object" && "message" in error) return String(error.message);
  return error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : translate("settingsPanels.agent.updateError");
}

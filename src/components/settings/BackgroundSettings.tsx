import { useEffect, useState } from "react";
import { useT } from "../../i18n";
import {
  type AppBackgroundPreference,
  type BackgroundImage,
  chooseBackgroundImage,
  readBackgroundImage,
  removeBackgroundImage,
  setStoredAppBackground,
  useAppBackground,
} from "../../lib/app-background";
import { SegmentedControl } from "../ui/SegmentedControl";

export function BackgroundSettings() {
  const t = useT();
  const preference = useAppBackground();
  const [image, setImage] = useState<BackgroundImage | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  useEffect(() => {
    let disposed = false;
    void readBackgroundImage()
      .then((next) => {
        if (!disposed) setImage(next);
      })
      .catch(() => {})
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, []);
  async function choose() {
    setBusy(true);
    setError(false);
    try {
      const next = await chooseBackgroundImage();
      if (next) {
        setImage(next);
        setStoredAppBackground("image");
      }
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  }
  async function remove() {
    setBusy(true);
    setError(false);
    try {
      await removeBackgroundImage();
      setImage(null);
      setStoredAppBackground("none");
    } catch {
      setError(true);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="settings-row">
      <div className="settings-row-info">
        <h3 className="settings-row-title">{t("settings.background.title")}</h3>
        <p className="settings-row-description">{t("settings.background.description")}</p>
        {preference === "image" && (
          <>
            {image && (
              <p className="settings-row-description background-file-name">{image.fileName}</p>
            )}
            {!loading && !image?.dataUrl && (
              <p className="settings-row-description">{t("settings.background.missing")}</p>
            )}
          </>
        )}
        {error && (
          <p role="alert" className="settings-row-description">
            {t("settings.background.error")}
          </p>
        )}
      </div>
      <div className="settings-row-control background-settings-controls">
        <fieldset disabled={busy || loading} className="background-settings-choice">
          <SegmentedControl<AppBackgroundPreference>
            aria-label={t("settings.background.title")}
            value={preference}
            options={[
              { value: "drift", label: t("settings.background.drift") },
              { value: "image", label: t("settings.background.image") },
              { value: "none", label: t("settings.background.none") },
            ]}
            onValueChange={(next) => {
              setError(false);
              if (next === "image" && !image?.dataUrl) void choose();
              else setStoredAppBackground(next);
            }}
          />
        </fieldset>
        {preference === "image" && (
          <div className="background-settings-actions">
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy || loading}
              onClick={() => void choose()}
            >
              {t("settings.background.choose")}
            </button>
            <button
              type="button"
              className="btn btn-ghost"
              disabled={busy || loading}
              onClick={() => void remove()}
            >
              {t("settings.background.remove")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

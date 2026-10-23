import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import { commands } from "../../bindings";
import { useSettings } from "../../hooks/useSettings";
import { ProgressBar } from "../shared";
import {
  hasAvailableUpdate,
  installAvailableUpdate,
  PERSONAL_RELEASES_URL,
} from "./updater";

interface UpdateCheckerProps {
  className?: string;
}

const UpdateChecker: React.FC<UpdateCheckerProps> = ({ className = "" }) => {
  const { t } = useTranslation();
  const { settings, isLoading } = useSettings();
  const settingsLoaded = !isLoading && settings !== null;
  const updateChecksEnabled = settings?.update_checks_enabled ?? false;
  const [isChecking, setIsChecking] = useState(false);
  const [updateAvailable, setUpdateAvailable] = useState(false);
  const [isInstalling, setIsInstalling] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [showUpToDate, setShowUpToDate] = useState(false);
  const [showPortableUpdateDialog, setShowPortableUpdateDialog] =
    useState(false);
  const busy = useRef(false);
  const upToDateTimeout = useRef<ReturnType<typeof setTimeout>>();

  const checkForUpdates = useCallback(async (manual = false) => {
    if (busy.current) return;
    busy.current = true;
    setIsChecking(true);
    try {
      const available = await hasAvailableUpdate();
      setUpdateAvailable(available);
      setShowUpToDate(manual && !available);
      if (upToDateTimeout.current) clearTimeout(upToDateTimeout.current);
      if (manual && !available) {
        upToDateTimeout.current = setTimeout(
          () => setShowUpToDate(false),
          3000,
        );
      }
    } catch (error) {
      console.error("Failed to check for Personal updates:", error);
      if (manual) toast.error(String(error));
    } finally {
      busy.current = false;
      setIsChecking(false);
    }
  }, []);

  useEffect(() => {
    if (settingsLoaded && updateChecksEnabled) void checkForUpdates();
  }, [settingsLoaded, updateChecksEnabled, checkForUpdates]);

  useEffect(() => {
    const unlisten = listen("check-for-updates", () => {
      void checkForUpdates(true);
    });
    return () => {
      if (upToDateTimeout.current) clearTimeout(upToDateTimeout.current);
      void unlisten.then((stop) => stop());
    };
  }, [checkForUpdates]);

  const installUpdate = async () => {
    if (busy.current) return;
    busy.current = true;
    setIsInstalling(true);
    setDownloadProgress(0);
    try {
      if (await commands.isPortable()) {
        setShowPortableUpdateDialog(true);
        return;
      }
      if (await installAvailableUpdate(setDownloadProgress)) {
        await relaunch();
      } else {
        setUpdateAvailable(false);
        setShowUpToDate(true);
      }
    } catch (error) {
      console.error("Failed to install Personal update:", error);
      toast.error(String(error));
    } finally {
      busy.current = false;
      setIsInstalling(false);
      setDownloadProgress(0);
    }
  };

  const status = isInstalling
    ? downloadProgress === 100
      ? t("footer.installing")
      : downloadProgress > 0
        ? t("footer.downloading", { progress: downloadProgress })
        : t("footer.preparing")
    : isChecking
      ? t("footer.checkingUpdates")
      : showUpToDate
        ? t("footer.upToDate")
        : updateAvailable
          ? t("footer.updateAvailableShort")
          : t("footer.checkForUpdates");

  return (
    <>
      {showPortableUpdateDialog && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div className="bg-background border border-mid-gray/20 rounded-lg p-6 max-w-md w-full mx-4 space-y-4">
            <h2 className="text-base font-semibold">
              {t("footer.portableUpdateTitle")}
            </h2>
            <p className="text-sm text-text/70">
              {t("footer.portableUpdateMessage")}
            </p>
            <div className="flex gap-2 justify-end">
              <button
                className="px-3 py-1.5 text-sm rounded border border-mid-gray/20 hover:bg-mid-gray/10 transition-colors"
                onClick={() => setShowPortableUpdateDialog(false)}
              >
                {t("common.close")}
              </button>
              <button
                className="px-3 py-1.5 text-sm rounded bg-logo-primary text-white hover:bg-logo-primary/80 transition-colors"
                onClick={() => {
                  void openUrl(PERSONAL_RELEASES_URL);
                  setShowPortableUpdateDialog(false);
                }}
              >
                {t("footer.portableUpdateButton")}
              </button>
            </div>
          </div>
        </div>
      )}
      <div className={`flex items-center gap-3 ${className}`}>
        <button
          onClick={() => {
            if (updateAvailable) void installUpdate();
            else void checkForUpdates(true);
          }}
          disabled={isChecking || isInstalling}
          className={`transition-colors disabled:opacity-50 tabular-nums ${
            updateAvailable
              ? "text-logo-primary hover:text-logo-primary/80 font-medium"
              : "text-text/60 hover:text-text/80"
          }`}
        >
          {status}
        </button>
        {isInstalling && downloadProgress > 0 && downloadProgress < 100 && (
          <ProgressBar
            progress={[{ id: "update", percentage: downloadProgress }]}
            size="large"
          />
        )}
      </div>
    </>
  );
};

export default UpdateChecker;

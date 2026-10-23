import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { Loader2, Upload } from "lucide-react";
import {
  runPanelCommand,
  usePanelTranscriptionStore,
} from "@/stores/panelTranscriptionStore";
import { isMediaFile, MEDIA_EXTENSIONS } from "@/lib/utils/transcription";
import { Button } from "../ui/Button";
import { TranscriptResult } from "./TranscriptResult";
import { TranscriptionStatus } from "./TranscriptionStatus";

export function FileTranscription() {
  const { t } = useTranslation();
  const { status, transcripts, pending, initialized, activeJob } =
    usePanelTranscriptionStore();
  const [dragging, setDragging] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [choosing, setChoosing] = useState(false);
  const dropZone = useRef<HTMLDivElement>(null);
  const busy = !initialized || pending || status.phase !== "idle";
  const transcribing = status.phase === "transcribing" && activeJob === "file";

  const transcribeFile = useCallback(
    async (path: string) => {
      const current = usePanelTranscriptionStore.getState();
      if (
        !current.initialized ||
        current.pending ||
        current.status.phase !== "idle"
      )
        return;
      if (!isMediaFile(path)) {
        setLocalError(t("fileTranscription.unsupportedFile"));
        return;
      }
      setLocalError(null);
      await runPanelCommand("transcribe_panel_file", { path });
    },
    [t],
  );

  useEffect(() => {
    let mounted = true;
    const unlisten = getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        if (!mounted) return;
        const current = usePanelTranscriptionStore.getState();
        if (
          payload.type === "leave" ||
          current.pending ||
          current.status.phase !== "idle"
        ) {
          setDragging(false);
          return;
        }
        const rect = dropZone.current?.getBoundingClientRect();
        const x = payload.position.x / window.devicePixelRatio;
        const y = payload.position.y / window.devicePixelRatio;
        const overZone =
          !!rect &&
          x >= rect.left &&
          x <= rect.right &&
          y >= rect.top &&
          y <= rect.bottom;
        if (payload.type === "drop") {
          setDragging(false);
          const path = payload.paths[0];
          if (overZone && path) void transcribeFile(path);
        } else {
          setDragging(overZone);
        }
      })
      .catch((error) => {
        if (mounted) setLocalError(String(error));
      });
    return () => {
      mounted = false;
      void unlisten.then((cleanup) => cleanup?.());
    };
  }, [transcribeFile]);

  useEffect(() => {
    if (transcribing) setLocalError(null);
  }, [transcribing]);

  const chooseFile = async () => {
    setLocalError(null);
    setChoosing(true);
    try {
      const selected = await open({
        title: t("sidebar.fileTranscription"),
        multiple: false,
        directory: false,
        filters: [
          {
            name: t("fileTranscription.mediaFiles"),
            extensions: MEDIA_EXTENSIONS,
          },
        ],
      });
      if (typeof selected === "string") await transcribeFile(selected);
    } catch (error) {
      setLocalError(String(error));
    } finally {
      setChoosing(false);
    }
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <h1 className="text-xl font-semibold">
        {t("sidebar.fileTranscription")}
      </h1>
      <div
        ref={dropZone}
        role="group"
        aria-label={t("fileTranscription.drop")}
        aria-disabled={busy || choosing}
        aria-busy={transcribing}
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => event.preventDefault()}
        className={`flex flex-col items-center justify-center gap-4 rounded-lg border-2 border-dashed px-6 py-12 transition-colors ${
          dragging && !busy
            ? "border-logo-primary bg-logo-primary/10"
            : "border-mid-gray/30 bg-mid-gray/5"
        }`}
      >
        {transcribing ? (
          <Loader2
            size={32}
            className="animate-spin motion-reduce:animate-none text-text/60"
            aria-hidden
          />
        ) : (
          <Upload size={32} className="text-text/60" aria-hidden />
        )}
        <p className="text-sm text-center text-text/60">
          {t(dragging && !busy ? "panel.dropNow" : "fileTranscription.drop")}
        </p>
        <Button
          variant="secondary"
          disabled={busy || choosing}
          onClick={chooseFile}
          className="focus-visible:ring-2 focus-visible:ring-logo-primary"
        >
          {t("fileTranscription.chooseFile")}
        </Button>
      </div>
      <TranscriptionStatus job="file" error={localError} />
      <TranscriptResult transcript={transcripts.file} />
    </div>
  );
}

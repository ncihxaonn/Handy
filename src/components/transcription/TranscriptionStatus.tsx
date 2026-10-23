import { useTranslation } from "react-i18next";
import {
  usePanelTranscriptionStore,
  type TranscriptionJob,
} from "@/stores/panelTranscriptionStore";

export function TranscriptionStatus({
  job,
  error: localError,
}: {
  job: TranscriptionJob;
  error?: string | null;
}) {
  const { t } = useTranslation();
  const {
    status,
    activeJob,
    transcripts,
    errors,
    initialized,
    connectionError,
  } = usePanelTranscriptionStore();
  const error = localError ?? errors[job] ?? connectionError;
  let message: string;

  if (error) message = t("transcription.error", { error });
  else if (!initialized) message = t("common.loading");
  else if (status.phase === "recording") {
    message = t(job === "meeting" ? "transcription.recording" : "panel.busy");
  } else if (status.phase === "transcribing") {
    message =
      activeJob !== job
        ? t("panel.busy")
        : status.file_name
          ? t("panel.transcribingFile", { name: status.file_name })
          : t("panel.transcribing");
    if (activeJob === job && status.progress) {
      message += ` ${status.progress.completed} / ${status.progress.total}`;
    }
  } else {
    message = t(
      transcripts[job] !== null ? "transcription.done" : "transcription.ready",
    );
  }

  return (
    <p
      role="status"
      aria-live="polite"
      aria-atomic="true"
      className={`text-sm break-words select-text ${error ? "text-red-500" : "text-text/60"}`}
    >
      {message}
    </p>
  );
}

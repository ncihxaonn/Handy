import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Mic, Square } from "lucide-react";
import {
  runPanelCommand,
  usePanelTranscriptionStore,
} from "@/stores/panelTranscriptionStore";
import { formatElapsed } from "@/lib/utils/transcription";
import { Button } from "../ui/Button";
import { TranscriptResult } from "./TranscriptResult";
import { TranscriptionStatus } from "./TranscriptionStatus";

export function MeetingRecording() {
  const { t } = useTranslation();
  const { status, transcripts, pending, initialized, activeJob } =
    usePanelTranscriptionStore();
  const [now, setNow] = useState(Date.now());
  const recording = status.phase === "recording";
  const transcribing =
    status.phase === "transcribing" && activeJob === "meeting";

  useEffect(() => {
    setNow(Date.now());
    if (!recording) return;
    const interval = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(interval);
  }, [recording, status.started_at_ms]);

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <h1 className="text-xl font-semibold">{t("sidebar.meetingRecording")}</h1>
      <div className="rounded-lg border border-mid-gray/20 bg-mid-gray/5 p-6 space-y-5">
        <div className="flex flex-col items-center gap-5">
          <p
            role="timer"
            aria-label={t("transcription.elapsedTime")}
            className="font-mono text-4xl leading-tight tabular-nums"
          >
            {formatElapsed(
              recording && status.started_at_ms !== null
                ? now - status.started_at_ms
                : 0,
            )}
          </p>
          <Button
            size="lg"
            variant={recording ? "danger" : "primary"}
            disabled={
              !initialized || pending || status.phase === "transcribing"
            }
            onClick={() =>
              runPanelCommand(
                recording ? "stop_panel_recording" : "start_panel_recording",
              )
            }
            className="flex min-h-16 w-full max-w-xs items-center justify-center gap-3 focus-visible:ring-2 focus-visible:ring-logo-primary"
          >
            {transcribing ? (
              <Loader2
                size={24}
                className="animate-spin motion-reduce:animate-none"
                aria-hidden
              />
            ) : recording ? (
              <Square size={24} aria-hidden />
            ) : (
              <Mic size={24} aria-hidden />
            )}
            {transcribing
              ? t("panel.transcribing")
              : recording
                ? t("meetingRecording.stopRecording")
                : t("panel.startRecording")}
          </Button>
          {(recording || transcribing) && (
            <Button
              variant="ghost"
              disabled={pending}
              onClick={() => runPanelCommand("cancel_panel_recording")}
              className="focus-visible:ring-2 focus-visible:ring-logo-primary"
            >
              {t(recording ? "panel.discard" : "common.cancel")}
            </Button>
          )}
        </div>
        <div className="text-center">
          <TranscriptionStatus job="meeting" />
        </div>
      </div>
      <TranscriptResult transcript={transcripts.meeting} savedToHistory />
    </div>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { exit } from "@tauri-apps/plugin-process";
import {
  Check,
  Copy,
  FileAudio,
  Loader2,
  Mic,
  Power,
  Settings,
  Square,
  Upload,
} from "lucide-react";
import { syncLanguageFromSettings } from "@/i18n";

type PanelPhase = "idle" | "recording" | "transcribing";

interface PanelStatus {
  phase: PanelPhase;
  started_at_ms: number | null;
  file_name: string | null;
  error: string | null;
}

type Notice = { kind: "ok" | "error"; text: string } | null;

const MEDIA_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "aac",
  "flac",
  "ogg",
  "opus",
  "aiff",
  "aif",
  "caf",
  "webm",
  "mp4",
  "mov",
  "mkv",
  "m4v",
];

const formatElapsed = (ms: number) => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
};

export default function MenubarPanel() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<PanelStatus>({
    phase: "idle",
    started_at_ms: null,
    file_name: null,
    error: null,
  });
  const [now, setNow] = useState(Date.now());
  const [lastTranscript, setLastTranscript] = useState<string | null>(null);
  const [modelName, setModelName] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const [copied, setCopied] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);
  const noticeTimer = useRef<number>();

  const showNotice = useCallback((next: Notice) => {
    window.clearTimeout(noticeTimer.current);
    setNotice(next);
    if (next) {
      noticeTimer.current = window.setTimeout(() => setNotice(null), 4000);
    }
  }, []);

  const refresh = useCallback(async () => {
    const [nextStatus, last, model] = await Promise.all([
      invoke<PanelStatus>("get_panel_status"),
      invoke<string | null>("get_last_transcript"),
      invoke<string | null>("get_selected_model_name"),
    ]);
    setStatus(nextStatus);
    setLastTranscript(last);
    setModelName(model);
  }, []);

  const runAction = useCallback(
    async (command: string, args?: Record<string, unknown>) => {
      try {
        await invoke(command, args);
      } catch (err) {
        showNotice({ kind: "error", text: String(err) });
      }
    },
    [showNotice],
  );

  const transcribeFile = useCallback(
    (path: string) => runAction("transcribe_panel_file", { path }),
    [runAction],
  );

  useEffect(() => {
    refresh();
    const unlisteners = [
      listen("panel-shown", () => {
        syncLanguageFromSettings();
        refresh();
      }),
      listen<PanelStatus>("panel-status-changed", (event) => {
        setStatus(event.payload);
        if (event.payload.error) {
          showNotice({ kind: "error", text: event.payload.error });
        }
      }),
      listen<string>("panel-transcript-ready", (event) => {
        setLastTranscript(event.payload);
        showNotice({ kind: "ok", text: t("panel.doneCopied") });
      }),
      listen("history-update-payload", () => refresh()),
      getCurrentWebview().onDragDropEvent((event) => {
        const { type } = event.payload;
        if (type === "enter" || type === "over") {
          setDragging(true);
        } else if (type === "leave") {
          setDragging(false);
        } else if (type === "drop") {
          setDragging(false);
          const [first] = event.payload.paths;
          if (first) transcribeFile(first);
        }
      }),
    ];
    return () => {
      unlisteners.forEach((p) => p.then((unlisten) => unlisten()));
    };
  }, [refresh, showNotice, t, transcribeFile]);

  useEffect(() => {
    if (status.phase !== "recording") return;
    const id = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(id);
  }, [status.phase]);

  const onRecordClick = () => {
    if (status.phase === "idle") runAction("start_panel_recording");
    else if (status.phase === "recording") runAction("stop_panel_recording");
  };

  const onPickFile = async () => {
    if (status.phase !== "idle") return;
    const selected = await open({
      multiple: false,
      filters: [{ name: "Audio / Video", extensions: MEDIA_EXTENSIONS }],
    });
    if (typeof selected === "string") transcribeFile(selected);
  };

  const onCopy = async () => {
    const ok = await invoke<boolean>("copy_last_transcript_command");
    if (ok) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } else {
      showNotice({ kind: "error", text: t("panel.nothingToCopy") });
    }
  };

  const busy = status.phase !== "idle";
  const recording = status.phase === "recording";
  const transcribing = status.phase === "transcribing";
  const elapsed = status.started_at_ms ? now - status.started_at_ms : 0;

  return (
    <div className="hp-shell">
      <header className="hp-header">
        <div className="hp-brand">
          <span className="hp-logo" aria-hidden />
          <span className="hp-title">{"Handy"}</span>
          {modelName && <span className="hp-chip">{modelName}</span>}
        </div>
        <button
          className="hp-icon-btn"
          onClick={() => runAction("open_main_window_from_panel")}
          title={t("panel.settings")}
          aria-label={t("panel.settings")}
        >
          <Settings size={16} />
        </button>
      </header>

      <button
        className={`hp-record ${recording ? "is-recording" : ""} ${transcribing ? "is-working" : ""}`}
        onClick={onRecordClick}
        disabled={transcribing}
      >
        <span className="hp-record-icon">
          {transcribing ? (
            <Loader2 size={20} className="hp-spin" />
          ) : recording ? (
            <Square size={16} fill="currentColor" />
          ) : (
            <Mic size={20} />
          )}
        </span>
        <span className="hp-record-text">
          <span className="hp-record-label">
            {transcribing
              ? status.file_name
                ? t("panel.transcribingFile", { name: status.file_name })
                : t("panel.transcribing")
              : recording
                ? t("panel.endRecording")
                : t("panel.startRecording")}
          </span>
          <span className="hp-record-sub">
            {recording
              ? t("panel.recordingHint")
              : transcribing
                ? t("panel.busy")
                : t("panel.idleHint")}
          </span>
        </span>
        {recording && (
          <span className="hp-timer">
            <span className="hp-dot" />
            {formatElapsed(elapsed)}
          </span>
        )}
      </button>
      {recording && (
        <button
          className="hp-link hp-discard"
          onClick={() => runAction("cancel_panel_recording")}
        >
          {t("panel.discard")}
        </button>
      )}

      <button
        className={`hp-drop ${dragging ? "is-dragging" : ""}`}
        onClick={onPickFile}
        disabled={busy}
      >
        {dragging ? <FileAudio size={22} /> : <Upload size={20} />}
        <span className="hp-drop-text">
          <span className="hp-drop-title">
            {dragging ? t("panel.dropNow") : t("panel.importTitle")}
          </span>
          {!dragging && (
            <span className="hp-drop-sub">{t("panel.importHint")}</span>
          )}
        </span>
      </button>

      <section className="hp-last">
        <div className="hp-last-head">
          <span>{t("panel.lastTranscript")}</span>
        </div>
        <p className={`hp-last-text ${lastTranscript ? "" : "is-empty"}`}>
          {lastTranscript || t("panel.noTranscript")}
        </p>
        <button className="hp-copy" onClick={onCopy} disabled={!lastTranscript}>
          {copied ? <Check size={15} /> : <Copy size={15} />}
          {copied ? t("panel.copied") : t("panel.copyLast")}
        </button>
      </section>

      <footer className="hp-footer">
        {notice ? (
          <span className={`hp-notice is-${notice.kind}`}>{notice.text}</span>
        ) : (
          <button
            className="hp-link"
            onClick={() => runAction("open_main_window_from_panel")}
          >
            {t("panel.openApp")}
          </button>
        )}
        <button
          className="hp-icon-btn"
          onClick={() => exit(0)}
          title={t("panel.quit")}
          aria-label={t("panel.quit")}
        >
          <Power size={14} />
        </button>
      </footer>
    </div>
  );
}

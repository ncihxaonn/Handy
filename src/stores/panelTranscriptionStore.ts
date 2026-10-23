import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type TranscriptionJob = "meeting" | "file";

export interface PanelStatus {
  phase: "idle" | "recording" | "transcribing";
  started_at_ms: number | null;
  file_name: string | null;
  error: string | null;
  progress?: { completed: number; total: number } | null;
}

interface PanelBackend {
  invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  listen: <T>(
    event: string,
    handler: (payload: T) => void,
  ) => Promise<() => void>;
}

interface PanelTranscriptionState {
  status: PanelStatus;
  activeJob: TranscriptionJob | null;
  transcripts: Record<TranscriptionJob, string | null>;
  errors: Record<TranscriptionJob, string | null>;
  initialized: boolean;
  pending: boolean;
  connectionError: string | null;
}

const backend: PanelBackend = {
  invoke,
  listen: <T>(event: string, handler: (payload: T) => void) =>
    listen<T>(event, ({ payload }) => handler(payload)),
};

export const createPanelTranscriptionSession = (
  bridge: PanelBackend = backend,
) => {
  const useStore = create<PanelTranscriptionState>(() => ({
    status: {
      phase: "idle",
      started_at_ms: null,
      file_name: null,
      error: null,
    },
    activeJob: null,
    transcripts: { meeting: null, file: null },
    errors: { meeting: null, file: null },
    initialized: false,
    pending: false,
    connectionError: null,
  }));

  let revision = 0;
  const updateStatus = (status: PanelStatus) => {
    revision += 1;
    useStore.setState((state) => {
      const activeJob =
        status.phase === "recording"
          ? "meeting"
          : status.phase === "transcribing"
            ? status.file_name
              ? "file"
              : "meeting"
            : state.activeJob;
      const next = { status, activeJob, initialized: true };
      if (activeJob && status.phase !== "idle") {
        return {
          ...next,
          transcripts: { ...state.transcripts, [activeJob]: null },
          errors: { ...state.errors, [activeJob]: null },
        };
      }
      if (status.error) {
        return {
          ...next,
          errors: activeJob
            ? { ...state.errors, [activeJob]: status.error }
            : { meeting: status.error, file: status.error },
          activeJob: null,
        };
      }
      return next;
    });
  };

  const connect = () => {
    let connected = true;
    const unlisteners: (() => void)[] = [];
    const subscribe = async <T>(
      event: string,
      handler: (payload: T) => void,
    ) => {
      const unlisten = await bridge.listen<T>(event, (payload) => {
        if (!connected) return;
        revision += 1;
        handler(payload);
      });
      if (connected) unlisteners.push(unlisten);
      else unlisten();
    };

    void Promise.all([
      subscribe<PanelStatus>("panel-status-changed", updateStatus),
      subscribe<string>("panel-transcript-ready", (transcript) => {
        useStore.setState((state) => {
          if (!state.activeJob) return state;
          return {
            transcripts: {
              ...state.transcripts,
              [state.activeJob]: transcript,
            },
            errors: { ...state.errors, [state.activeJob]: null },
            activeJob: null,
          };
        });
      }),
    ])
      .then(async () => {
        if (!connected) return;
        const beforeRequest = revision;
        const status = (await bridge.invoke("get_panel_status")) as PanelStatus;
        if (connected && revision === beforeRequest) updateStatus(status);
        if (connected)
          useStore.setState({ initialized: true, connectionError: null });
      })
      .catch((error) => {
        if (connected) useStore.setState({ connectionError: String(error) });
      });

    return () => {
      connected = false;
      unlisteners.forEach((unlisten) => unlisten());
    };
  };

  const runCommand = async (
    command:
      | "start_panel_recording"
      | "stop_panel_recording"
      | "cancel_panel_recording"
      | "transcribe_panel_file",
    args?: Record<string, unknown>,
  ) => {
    const state = useStore.getState();
    if (!state.initialized || state.pending) return;
    const isStarting =
      command === "start_panel_recording" ||
      command === "transcribe_panel_file";
    if (
      isStarting
        ? state.status.phase !== "idle"
        : command === "cancel_panel_recording"
          ? state.status.phase === "idle"
          : state.status.phase !== "recording"
    ) {
      return;
    }
    const job = isStarting
      ? command === "transcribe_panel_file"
        ? "file"
        : "meeting"
      : (state.activeJob ?? "meeting");
    useStore.setState({
      pending: true,
      errors: { ...state.errors, [job]: null },
    });
    try {
      await bridge.invoke(command, args);
      const beforeRequest = revision;
      const status = (await bridge.invoke("get_panel_status")) as PanelStatus;
      if (revision === beforeRequest) updateStatus(status);
      if (command === "cancel_panel_recording") {
        useStore.setState((current) => ({
          activeJob: current.activeJob === job ? null : current.activeJob,
          transcripts: { ...current.transcripts, [job]: null },
        }));
      }
    } catch (error) {
      useStore.setState((current) => ({
        errors: { ...current.errors, [job]: String(error) },
      }));
    } finally {
      useStore.setState({ pending: false });
    }
  };

  return { useStore, connect, runCommand };
};

export const {
  useStore: usePanelTranscriptionStore,
  connect: connectPanelTranscription,
  runCommand: runPanelCommand,
} = createPanelTranscriptionSession();

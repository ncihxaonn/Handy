import { expect, test } from "bun:test";

type PayloadHandler = (payload: unknown) => void;

class PanelBackend {
  status = {
    phase: "idle",
    started_at_ms: null as number | null,
    file_name: null as string | null,
    error: null as string | null,
  };
  calls: { command: string; args?: Record<string, unknown> }[] = [];
  listeners = new Map<string, Set<PayloadHandler>>();

  invoke = async (command: string, args?: Record<string, unknown>) => {
    this.calls.push({ command, args });
    if (command === "get_panel_status") return this.status;
    if (command === "start_panel_recording") {
      this.emit("panel-status-changed", {
        ...this.status,
        phase: "recording",
        started_at_ms: 123000,
        error: null,
      });
    } else if (command === "stop_panel_recording") {
      this.emit("panel-status-changed", {
        ...this.status,
        phase: "transcribing",
        started_at_ms: null,
      });
    } else if (command === "cancel_panel_recording") {
      this.emit("panel-status-changed", {
        phase: "idle",
        started_at_ms: null,
        file_name: null,
        error: null,
      });
    }
    return null;
  };

  listen = async <T>(event: string, handler: (payload: T) => void) => {
    const callback: PayloadHandler = (payload) => handler(payload as T);
    const listeners = this.listeners.get(event) ?? new Set<PayloadHandler>();
    listeners.add(callback);
    this.listeners.set(event, listeners);
    return () => {
      listeners.delete(callback);
    };
  };

  emit(event: string, payload: unknown) {
    if (event === "panel-status-changed") {
      this.status = payload as typeof this.status;
    }
    this.listeners.get(event)?.forEach((handler) => handler(payload));
  }
}

const loadSession = async () => {
  const implementation = await import(
    "../src/stores/panelTranscriptionStore"
  ).catch(() => null);
  expect(implementation?.createPanelTranscriptionSession).toBeFunction();
  return implementation!.createPanelTranscriptionSession;
};

const settle = () => Bun.sleep(0);

test("meeting follows panel recording, transcribing and full transcript events", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();
  expect(session.useStore.getState().initialized).toBe(true);

  backend.emit("panel-status-changed", {
    phase: "recording",
    started_at_ms: 123000,
    file_name: null,
    error: null,
  });
  expect(session.useStore.getState().status.started_at_ms).toBe(123000);
  expect(session.useStore.getState().activeJob).toBe("meeting");

  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "transcribing",
    started_at_ms: null,
  });
  expect(session.useStore.getState().status.phase).toBe("transcribing");
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "idle",
  });
  const transcript = "Meeting notes\n" + "Full transcript. ".repeat(500);
  backend.emit("panel-transcript-ready", transcript);
  expect(session.useStore.getState().transcripts.meeting).toBe(transcript);
  expect(session.useStore.getState().transcripts.file).toBeNull();
  disconnect();
});

test("meeting controls reuse recording commands and discard clears the result", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();

  await session.runCommand("start_panel_recording");
  expect(session.useStore.getState().status.phase).toBe("recording");
  await session.runCommand("stop_panel_recording");
  expect(session.useStore.getState().status.phase).toBe("transcribing");
  backend.emit("panel-status-changed", { ...backend.status, phase: "idle" });
  backend.emit("panel-transcript-ready", "First meeting");
  await session.runCommand("start_panel_recording");
  await session.runCommand("cancel_panel_recording");

  expect(session.useStore.getState().status.phase).toBe("idle");
  expect(session.useStore.getState().transcripts.meeting).toBeNull();
  expect(session.useStore.getState().activeJob).toBeNull();
  expect(backend.calls.map(({ command }) => command)).toContain(
    "cancel_panel_recording",
  );
  disconnect();
});

test.each(["file", "meeting"] as const)(
  "canceling a %s transcription preserves the other job's result",
  async (job) => {
    const createSession = await loadSession();
    const backend = new PanelBackend();
    const session = createSession(backend);
    const disconnect = session.connect();
    await settle();
    session.useStore.setState({
      transcripts: { meeting: "Meeting result", file: "File result" },
    });
    backend.emit("panel-status-changed", {
      ...backend.status,
      phase: "transcribing",
      file_name: job === "file" ? "long.mp4" : null,
      progress: { completed: 2, total: 7 },
    });

    expect(session.useStore.getState().status.progress).toEqual({
      completed: 2,
      total: 7,
    });
    const invoke = backend.invoke;
    backend.invoke = async (command, args) => {
      if (command === "cancel_panel_recording") {
        backend.emit("panel-transcript-ready", "Canceled result");
      }
      return invoke(command, args);
    };

    await session.runCommand("cancel_panel_recording");

    expect(backend.calls.map(({ command }) => command)).toContain(
      "cancel_panel_recording",
    );
    expect(session.useStore.getState().status.phase).toBe("idle");
    expect(session.useStore.getState().activeJob).toBeNull();
    expect(session.useStore.getState().transcripts).toEqual({
      meeting: job === "meeting" ? null : "Meeting result",
      file: job === "file" ? null : "File result",
    });
    backend.emit("panel-transcript-ready", "Canceled result");
    expect(session.useStore.getState().transcripts[job]).toBeNull();
    disconnect();
  },
);

test.each(["file", "meeting"] as const)(
  "a failed %s transcription cancel reports the error to its active job",
  async (job) => {
    const createSession = await loadSession();
    const backend = new PanelBackend();
    const session = createSession(backend);
    const disconnect = session.connect();
    await settle();
    backend.emit("panel-status-changed", {
      ...backend.status,
      phase: "transcribing",
      file_name: job === "file" ? "long.mp4" : null,
    });
    const otherJob = job === "file" ? "meeting" : "file";
    session.useStore.setState({
      errors: {
        ...session.useStore.getState().errors,
        [otherJob]: "Other error",
      },
    });
    backend.invoke = async () => {
      throw new Error("Cancel failed");
    };

    await session.runCommand("cancel_panel_recording");

    expect(session.useStore.getState().errors[job]).toContain("Cancel failed");
    expect(session.useStore.getState().errors[otherJob]).toBe("Other error");
    expect(session.useStore.getState().activeJob).toBe(job);
    expect(session.useStore.getState().pending).toBe(false);
    disconnect();
  },
);

test("a delayed command refresh cannot overwrite a newer panel completion", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();

  const invoke = backend.invoke;
  let releaseStatus!: (status: unknown) => void;
  backend.invoke = async (command, args) => {
    if (command === "get_panel_status") {
      return new Promise((resolve) => {
        releaseStatus = resolve;
      });
    }
    return invoke(command, args);
  };
  const starting = session.runCommand("start_panel_recording");
  await settle();
  const staleStatus = { ...backend.status };
  backend.emit("panel-status-changed", { ...backend.status, phase: "idle" });
  backend.emit("panel-transcript-ready", "Completed while refresh was pending");
  releaseStatus(staleStatus);
  await starting;

  expect(session.useStore.getState().status.phase).toBe("idle");
  expect(session.useStore.getState().transcripts.meeting).toBe(
    "Completed while refresh was pending",
  );
  disconnect();
});

test("file jobs keep meeting and file results separate", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "recording",
  });
  backend.emit("panel-status-changed", { ...backend.status, phase: "idle" });
  backend.emit("panel-transcript-ready", "Meeting result");

  const path = "/Users/test/Audio recording.M4A";
  await session.runCommand("transcribe_panel_file", { path });
  expect(backend.calls).toContainEqual({
    command: "transcribe_panel_file",
    args: { path },
  });
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "transcribing",
    file_name: "Audio recording.M4A",
  });
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "idle",
    file_name: null,
  });
  backend.emit("panel-transcript-ready", "File result");
  expect(session.useStore.getState().transcripts).toEqual({
    meeting: "Meeting result",
    file: "File result",
  });
  disconnect();
});

test("command errors are shown and controls unlock for retry", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();
  const invoke = backend.invoke;
  backend.invoke = async (command, args) => {
    if (command === "start_panel_recording")
      throw new Error("Microphone unavailable");
    return invoke(command, args);
  };
  await session.runCommand("start_panel_recording");
  expect(session.useStore.getState().errors.meeting).toContain(
    "Microphone unavailable",
  );
  expect(session.useStore.getState().pending).toBe(false);
  backend.invoke = invoke;
  await session.runCommand("start_panel_recording");
  expect(session.useStore.getState().errors.meeting).toBeNull();
  expect(session.useStore.getState().status.phase).toBe("recording");
  disconnect();
});

test("busy jobs reject overlapping starts without invoking the backend", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();
  await session.runCommand("start_panel_recording");
  const before = backend.calls.length;
  await session.runCommand("start_panel_recording");
  await session.runCommand("transcribe_panel_file", { path: "/test.mp4" });
  expect(backend.calls.length).toBe(before);
  disconnect();
});

test("opening the main window picks up a meeting already recording in the panel", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  backend.status = {
    ...backend.status,
    phase: "recording",
    started_at_ms: 123000,
  };
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();
  expect(session.useStore.getState().status).toEqual(backend.status);
  expect(session.useStore.getState().activeJob).toBe("meeting");
  disconnect();
});

test("disconnecting before async subscriptions settle removes every listener", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  disconnect();
  await settle();
  expect(
    [...backend.listeners.values()].every((listeners) => listeners.size === 0),
  ).toBe(true);
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "recording",
  });
  expect(session.useStore.getState().status.phase).toBe("idle");
});

test("backend file failures are scoped to the file page", async () => {
  const createSession = await loadSession();
  const backend = new PanelBackend();
  const session = createSession(backend);
  const disconnect = session.connect();
  await settle();
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "transcribing",
    file_name: "broken.mp4",
  });
  backend.emit("panel-status-changed", {
    ...backend.status,
    phase: "idle",
    file_name: null,
    error: "Decoder failed",
  });
  expect(session.useStore.getState().errors).toEqual({
    meeting: null,
    file: "Decoder failed",
  });
  expect(session.useStore.getState().status.phase).toBe("idle");
  disconnect();
});

import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type {
  PanelStatus,
  TranscriptionJob,
} from "../src/stores/panelTranscriptionStore";

mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const {
  createPanelTranscriptionSession,
  connectPanelTranscription,
  runPanelCommand,
  usePanelTranscriptionStore: store,
} = await import("../src/stores/panelTranscriptionStore");

mock.module("../src/stores/panelTranscriptionStore", () => ({
  createPanelTranscriptionSession,
  connectPanelTranscription,
  runPanelCommand,
  usePanelTranscriptionStore: Object.assign(() => store.getState(), {
    getState: store.getState,
  }),
}));

const { TranscriptionStatus } = await import(
  "../src/components/transcription/TranscriptionStatus"
);
const { MeetingRecording } = await import(
  "../src/components/transcription/MeetingRecording"
);
const { FileTranscription } = await import(
  "../src/components/transcription/FileTranscription"
);

const setTranscribing = (
  job: TranscriptionJob,
  status?: Partial<PanelStatus>,
) =>
  store.setState({
    initialized: true,
    activeJob: job,
    status: {
      phase: "transcribing",
      started_at_ms: null,
      file_name: job === "file" ? "long.mp4" : null,
      error: null,
      progress: { completed: 2, total: 7 },
      ...status,
    },
  });

afterEach(() => store.setState(store.getInitialState(), true));

test.each(["file", "meeting"] as const)(
  "%s transcription status announces completed segments",
  (job) => {
    setTranscribing(job);

    const html = renderToStaticMarkup(<TranscriptionStatus job={job} />);

    expect(html).toContain('role="status"');
    expect(html).toContain('aria-live="polite"');
    expect(html).toContain("2 / 7");
  },
);

test.each([
  ["meeting", MeetingRecording],
  ["file", FileTranscription],
] as const)("the %s page exposes cancel while transcribing", (job, Page) => {
  setTranscribing(job);

  const html = renderToStaticMarkup(<Page />);
  const cancelButton = html
    .match(/<button\b[^>]*>[\s\S]*?<\/button>/g)
    ?.find((button) => button.includes("common.cancel"));

  expect(cancelButton).toBeDefined();
  expect(cancelButton).not.toContain('disabled=""');
  expect(html).toContain("2 / 7");
});

test.each([
  ["meeting", MeetingRecording],
  ["file", FileTranscription],
] as const)("the %s page cannot cancel the other job", (job, Page) => {
  setTranscribing(job === "meeting" ? "file" : "meeting");

  const html = renderToStaticMarkup(<Page />);

  expect(html).not.toContain("common.cancel");
  expect(html).not.toContain("2 / 7");
});

test("an unrelated page stays busy without another job's progress", () => {
  setTranscribing("file");

  const html = renderToStaticMarkup(<TranscriptionStatus job="meeting" />);

  expect(html).toContain("panel.busy");
  expect(html).not.toContain("2 / 7");
});

test("older statuses without segment progress still render", () => {
  setTranscribing("meeting", { progress: undefined });

  const html = renderToStaticMarkup(<TranscriptionStatus job="meeting" />);

  expect(html).toContain("panel.transcribing");
  expect(html).not.toContain(" / ");
});

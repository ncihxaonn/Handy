import { afterEach, expect, mock, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
let checksEnabled = true;
const checkUpdate = mock(async (): Promise<unknown> => null);
const closeUpdate = mock(async () => {});
mock.module("@tauri-apps/plugin-updater", () => ({ check: checkUpdate }));

mock.module("../src/hooks/useSettings", () => ({
  useSettings: () => ({
    settings: { update_checks_enabled: checksEnabled },
    isLoading: false,
    getSetting: () => checksEnabled,
    updateSetting: () => {},
    isUpdating: () => false,
  }),
}));

const { default: UpdateChecker } = await import(
  "../src/components/update-checker/UpdateChecker"
);

afterEach(() => {
  checksEnabled = true;
  checkUpdate.mockReset();
  checkUpdate.mockResolvedValue(null);
  closeUpdate.mockClear();
});

test("the footer exposes the Personal update check", () => {
  const html = renderToStaticMarkup(<UpdateChecker />);
  expect(html).toContain("footer.checkForUpdates");
  expect(html).toContain("<button");
  expect(html).not.toContain('disabled=""');
});

const { UpdateChecksToggle } = await import(
  "../src/components/settings/UpdateChecksToggle"
);

test("the update toggle respects an enabled Personal update setting", () => {
  const html = renderToStaticMarkup(<UpdateChecksToggle />);
  const input = html.match(/<input\b[^>]*>/)?.[0] ?? "";
  expect(input).toContain('type="checkbox"');
  expect(input).not.toContain('disabled=""');
  expect(input).toContain('checked=""');
});

test("manual checks remain available when automatic checks are disabled", () => {
  checksEnabled = false;
  const html = renderToStaticMarkup(<UpdateChecker />);
  const toggle = renderToStaticMarkup(<UpdateChecksToggle />);
  expect(html).toContain("footer.checkForUpdates");
  expect(html).not.toContain('disabled=""');
  expect(toggle.match(/<input\b[^>]*>/)?.[0]).not.toContain('checked=""');
});

const { hasAvailableUpdate, installAvailableUpdate, PERSONAL_RELEASES_URL } =
  await import("../src/components/update-checker/updater");

test("availability checks close their native update resource", async () => {
  checkUpdate.mockResolvedValue({ close: closeUpdate });
  expect(await hasAvailableUpdate()).toBe(true);
  expect(checkUpdate).toHaveBeenCalledTimes(1);
  expect(closeUpdate).toHaveBeenCalledTimes(1);
});

test("an up-to-date availability check has no resource to close", async () => {
  expect(await hasAvailableUpdate()).toBe(false);
  expect(closeUpdate).not.toHaveBeenCalled();
});

test("the portable update link points only to the Personal fork", () => {
  expect(PERSONAL_RELEASES_URL).toBe(
    "https://github.com/ncihxaonn/Handy/releases/latest",
  );
});

test("no release means there is nothing to install", async () => {
  const progress = mock(() => {});
  expect(await installAvailableUpdate(progress)).toBe(false);
  expect(checkUpdate).toHaveBeenCalledTimes(1);
  expect(progress).not.toHaveBeenCalled();
});

test("the native updater downloads, verifies and installs with progress", async () => {
  const progress: number[] = [];
  const downloadAndInstall = mock(async (report: (event: unknown) => void) => {
    report({ event: "Started", data: { contentLength: 100 } });
    report({ event: "Progress", data: { chunkLength: 40 } });
    report({ event: "Progress", data: { chunkLength: 60 } });
    report({ event: "Finished" });
  });
  checkUpdate.mockResolvedValue({ downloadAndInstall, close: closeUpdate });
  expect(await installAvailableUpdate((value) => progress.push(value))).toBe(
    true,
  );
  expect(downloadAndInstall).toHaveBeenCalledTimes(1);
  expect(closeUpdate).toHaveBeenCalledTimes(1);
  expect(progress).toEqual([40, 100, 100]);
});

test("downloads without a content length still finish at 100 percent", async () => {
  const progress: number[] = [];
  checkUpdate.mockResolvedValue({
    close: closeUpdate,
    downloadAndInstall: async (report: (event: unknown) => void) => {
      report({ event: "Started", data: {} });
      report({ event: "Progress", data: { chunkLength: 100 } });
      report({ event: "Finished" });
    },
  });
  expect(await installAvailableUpdate((value) => progress.push(value))).toBe(
    true,
  );
  expect(progress).toEqual([0, 100]);
  expect(closeUpdate).toHaveBeenCalledTimes(1);
});

test("signature failures are not reported as successful installation", async () => {
  checkUpdate.mockResolvedValue({
    close: closeUpdate,
    downloadAndInstall: async () => {
      throw new Error("Invalid update signature");
    },
  });
  await expect(installAvailableUpdate(() => {})).rejects.toThrow(
    "Invalid update signature",
  );
  expect(closeUpdate).toHaveBeenCalledTimes(1);
});

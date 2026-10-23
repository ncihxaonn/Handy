import { check } from "@tauri-apps/plugin-updater";

export const PERSONAL_RELEASES_URL =
  "https://github.com/ncihxaonn/Handy/releases/latest";

export const hasAvailableUpdate = async (): Promise<boolean> => {
  const update = await check();
  if (!update) return false;
  await update.close();
  return true;
};

export const installAvailableUpdate = async (
  onProgress: (percentage: number) => void,
): Promise<boolean> => {
  const update = await check();
  if (!update) return false;

  let downloaded = 0;
  let total = 0;
  try {
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") {
        downloaded = 0;
        total = event.data.contentLength ?? 0;
      } else if (event.event === "Progress") {
        downloaded += event.data.chunkLength;
        onProgress(
          total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : 0,
        );
      } else if (event.event === "Finished") {
        onProgress(100);
      }
    });
    return true;
  } finally {
    await update.close();
  }
};

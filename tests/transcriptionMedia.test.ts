import { expect, test } from "bun:test";

const loadUtils = async () => {
  const implementation = await import("../src/lib/utils/transcription").catch(
    () => null,
  );
  expect(implementation).not.toBeNull();
  return implementation!;
};

test("media validation accepts audio and video paths without changing them", async () => {
  const { isMediaFile } = await loadUtils();
  for (const path of [
    "/Users/test/Audio recording.m4a",
    "/Users/test/MOVIE.MOV",
    "C:\\Meeting\\recording.MP3",
    "/recordings/clip.mp4",
    "/recordings/clip.wav",
    "/recordings/clip.flac",
    "/recordings/clip.webm",
  ]) {
    expect(isMediaFile(path)).toBe(true);
  }
  for (const path of [
    "/folder/mp3",
    "/clip.mp4.txt",
    "/file.pdf",
    "/folder.wav/no-extension",
  ]) {
    expect(isMediaFile(path)).toBe(false);
  }
});

test("meeting elapsed time supports long recordings and clamps clock skew", async () => {
  const { formatElapsed } = await loadUtils();
  expect(formatElapsed(-1000)).toBe("00:00");
  expect(formatElapsed(0)).toBe("00:00");
  expect(formatElapsed(65000)).toBe("01:05");
  expect(formatElapsed(3601000)).toBe("1:00:01");
});

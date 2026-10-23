export const MEDIA_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "m4b",
  "aac",
  "flac",
  "ogg",
  "oga",
  "opus",
  "aiff",
  "aif",
  "caf",
  "wma",
  "webm",
  "mp4",
  "mov",
  "mkv",
  "m4v",
  "avi",
  "mpeg",
  "mpg",
  "3gp",
];

export const isMediaFile = (path: string) => {
  const name = path.split(/[\\/]/).pop() ?? "";
  const extension = name.includes(".")
    ? name.split(".").pop()?.toLowerCase()
    : "";
  return MEDIA_EXTENSIONS.includes(extension ?? "");
};

export const formatElapsed = (ms: number) => {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = String(Math.floor((seconds % 3600) / 60)).padStart(2, "0");
  const remainder = String(seconds % 60).padStart(2, "0");
  return hours > 0
    ? `${hours}:${minutes}:${remainder}`
    : `${minutes}:${remainder}`;
};

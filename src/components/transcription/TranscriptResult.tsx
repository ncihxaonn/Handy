import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy } from "lucide-react";
import { Button } from "../ui/Button";
import { Textarea } from "../ui/Textarea";

interface TranscriptResultProps {
  transcript: string | null;
  savedToHistory?: boolean;
}

export function TranscriptResult({
  transcript,
  savedToHistory = false,
}: TranscriptResultProps) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);

  useEffect(() => {
    setCopied(false);
    setCopyError(null);
  }, [transcript]);

  useEffect(() => {
    if (!copied) return;
    const timeout = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(timeout);
  }, [copied]);

  if (transcript === null) return null;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(transcript);
      setCopyError(null);
      setCopied(true);
    } catch (error) {
      setCopyError(String(error));
    }
  };

  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-sm font-medium">{t("transcription.transcript")}</h2>
        <Button
          variant="secondary"
          size="sm"
          onClick={copy}
          className="flex items-center gap-2 focus-visible:ring-2 focus-visible:ring-logo-primary"
        >
          {copied ? (
            <Check size={14} aria-hidden />
          ) : (
            <Copy size={14} aria-hidden />
          )}
          {copied ? t("panel.copied") : t("common.copy")}
        </Button>
      </div>
      <Textarea
        readOnly
        aria-label={t("transcription.transcript")}
        value={transcript}
        rows={10}
        className="w-full max-h-96 overflow-y-auto resize-none font-normal select-text"
      />
      {savedToHistory && (
        <p className="text-sm text-text/60">
          {t("transcription.savedToHistory")}
        </p>
      )}
      {copyError && (
        <p role="alert" className="text-sm text-red-500 select-text">
          {t("transcription.copyFailed", { error: copyError })}
        </p>
      )}
    </section>
  );
}

import { expect, mock, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
mock.module("../src/i18n", () => ({
  syncLanguageFromSettings: async () => {},
}));

const { default: MenubarPanel } = await import("../src/panel/MenubarPanel");

test("the pin button starts unpinned before the initial backend refresh", () => {
  const html = renderToStaticMarkup(<MenubarPanel />);
  const pinButton =
    html.match(/<button\b[^>]*aria-label="panel.pin"[^>]*>/)?.[0] ?? "";
  expect(pinButton).toContain('aria-pressed="false"');
  expect(pinButton).toContain('title="panel.pin"');
});

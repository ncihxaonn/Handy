import { expect, mock, test } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

mock.module("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
mock.module("../src/hooks/useSettings", () => ({
  useSettings: () => ({
    settings: { update_checks_enabled: true },
    isLoading: false,
    getSetting: () => true,
    updateSetting: () => {},
    isUpdating: () => false,
  }),
}));

const { default: UpdateChecker } = await import(
  "../src/components/update-checker/UpdateChecker"
);

test("official updates stay disabled with an existing enabled setting", () => {
  const html = renderToStaticMarkup(<UpdateChecker />);
  expect(html).toContain("footer.updateCheckingDisabled");
  expect(html).not.toContain("<button");
});

const { UpdateChecksToggle } = await import(
  "../src/components/settings/UpdateChecksToggle"
);

test("the update toggle cannot re-enable official updates", () => {
  const html = renderToStaticMarkup(<UpdateChecksToggle />);
  const input = html.match(/<input\b[^>]*>/)?.[0] ?? "";
  expect(input).toContain('type="checkbox"');
  expect(input).toContain('disabled=""');
  expect(input).not.toContain('checked=""');
});

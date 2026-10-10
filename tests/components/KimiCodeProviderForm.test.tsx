import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { KimiCodeProviderForm } from "@/components/providers/forms/KimiCodeProviderForm";
import { kimiCodeProviderPresets } from "@/config/kimiCodeProviderPresets";

vi.mock("@/components/JsonEditor", () => ({
  default: ({
    id,
    value,
    onChange,
  }: {
    id: string;
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      id={id}
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

vi.mock("@/components/providers/forms/EndpointSpeedTest", () => ({
  default: () => null,
}));

const original = {
  type: "kimi",
  base_url: "https://api.moonshot.ai/v1",
  api_key: "local-test-key",
  models: {
    "kimi-for-coding": {
      model: "kimi-for-coding",
      max_context_size: 262144,
    },
  },
  futureSetting: { keep: true },
};

describe("KimiCodeProviderForm", () => {
  it("edits native fields while preserving unknown settings", async () => {
    const submit = vi.fn();
    render(
      <KimiCodeProviderForm
        appId="kimicode"
        providerId="existing"
        initialData={{ name: "Existing", settingsConfig: original }}
        submitLabel="Save"
        onSubmit={submit}
        onCancel={() => {}}
      />,
    );
    fireEvent.change(screen.getByLabelText("provider.name"), {
      target: { value: "Renamed" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(submit).toHaveBeenCalledOnce());
    const saved = JSON.parse(submit.mock.calls[0][0].settingsConfig);
    expect(saved).toEqual(original);
    expect(submit.mock.calls[0][0].providerKey).toBe("existing");
    expect(screen.getByLabelText("API Key")).toHaveAttribute(
      "type",
      "password",
    );
  });

  it("keeps a rejected save open and displays the failure", async () => {
    render(
      <KimiCodeProviderForm
        appId="kimicode"
        initialData={{ name: "Existing", settingsConfig: original }}
        submitLabel="Save"
        onSubmit={async () => {
          throw new Error("configuration busy");
        }}
        onCancel={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "configuration busy",
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("applies the Kimi Official preset and saves contract fields", async () => {
    const submit = vi.fn();
    render(
      <KimiCodeProviderForm
        appId="kimicode"
        submitLabel="Save"
        onSubmit={submit}
        onCancel={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Kimi Official/ }));
    fireEvent.change(screen.getByLabelText("API Key"), {
      target: { value: "sk-kimi-test" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(submit).toHaveBeenCalledOnce());
    const saved = JSON.parse(submit.mock.calls[0][0].settingsConfig);
    expect(saved.type).toBe("kimi");
    expect(saved.base_url).toBe("https://api.moonshot.ai/v1");
    expect(saved.api_key).toBe("sk-kimi-test");
    expect(saved.models["kimi-for-coding"]).toEqual({
      model: "kimi-for-coding",
      max_context_size: 262144,
    });
    expect(saved.models.k3).toEqual({
      model: "k3",
      max_context_size: 1048576,
    });
  });

  it("keeps invalid JSON drafts from breaking the structured fields", () => {
    render(
      <KimiCodeProviderForm
        appId="kimicode"
        initialData={{ name: "Existing", settingsConfig: original }}
        submitLabel="Save"
        onSubmit={vi.fn()}
        onCancel={() => {}}
      />,
    );
    fireEvent.change(screen.getByLabelText("provider.configJson"), {
      target: { value: '{"base_url":42' },
    });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("provider.configJson"), {
      target: { value: JSON.stringify(original) },
    });
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("declares presets following the settings_config contract", () => {
    const official = kimiCodeProviderPresets.find(
      (preset) => preset.name === "Kimi Official",
    )!;
    expect(official.settingsConfig.type).toBe("kimi");
    expect(official.settingsConfig.base_url).toBe("https://api.moonshot.ai/v1");
    expect(
      official.settingsConfig.models["kimi-for-coding"]?.max_context_size,
    ).toBe(262144);
    expect(official.settingsConfig.models.k3?.max_context_size).toBe(1048576);

    const anthropic = kimiCodeProviderPresets.find(
      (preset) => preset.settingsConfig.type === "anthropic",
    )!;
    expect(anthropic).toBeDefined();
    expect(
      Object.values(anthropic.settingsConfig.models).every(
        (model) =>
          typeof model.model === "string" &&
          Number.isInteger(model.max_context_size) &&
          model.max_context_size > 0,
      ),
    ).toBe(true);
  });
});

<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { DisplayMode, DisplayStatus, Settings } from "@/types/settings";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Select from "@/components/ui/Select.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  let { display = $bindable() }: { display: Settings["display"] } = $props();

  const SCALING = [
    { label: "Fit", value: "fit" },
    { label: "Fill", value: "fill" },
  ];
  const PORTS = [
    { label: "Both HDMI ports", value: "both" },
    { label: "HDMI 0", value: "hdmi0" },
    { label: "HDMI 1", value: "hdmi1" },
  ];

  let status = $state<DisplayStatus | null>(null);
  let loadError = $state<string | null>(null);

  onMount(() => {
    let active = true;
    let loading = false;
    async function refresh() {
      if (loading) return;
      loading = true;
      try {
        const next = await api.display();
        if (active) {
          status = next;
          loadError = null;
        }
      } catch (e) {
        if (active) loadError = message(e);
      } finally {
        loading = false;
      }
    }
    void refresh();
    const timer = setInterval(refresh, 2000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  });

  const hz = (r: number) =>
    Math.abs(r - Math.round(r)) < 0.02 ? String(Math.round(r)) : r.toFixed(2);

  const modeLabel = (m: DisplayMode) =>
    `${m.width} × ${m.height}${m.interlaced ? "i" : "p"} · ${hz(m.refresh)} Hz`;

  const modes = $derived.by(() => {
    const list = (status?.modes ?? []).map((m) => {
      const notes = [
        m.preferred && "screen's choice",
        m.manual && !m.advertised && "manual preset",
        m.advertised && !m.fits_screen && "different aspect ratio (EDID)",
        m.width >= 3840 && "may stutter",
      ].filter(Boolean);
      return {
        value: m.id,
        label: `${modeLabel(m)}${notes.length ? ` (${notes.join(", ")})` : ""}`,
      };
    });
    // A mode saved while another screen was plugged in
    if (display.mode !== "auto" && !list.some((m) => m.value === display.mode))
      list.unshift({
        value: display.mode,
        label: `${display.mode.replace("x", " × ").replace("@", " · ")} Hz (not offered by this screen)`,
      });
    return [{ value: "auto", label: "Automatic (best for video)" }, ...list];
  });

  const modeHelp = $derived(
    status?.error ??
      loadError ??
      "p = progressive; i = interlaced (Hz means fields per second). Manual presets use standard timings even if missing from EDID. Automatic uses screen-reported progressive modes. 4K presets are limited to 30 Hz.",
  );
</script>

<Card title="HDMI output">
  <Field
    label="Show video on HDMI"
    for="hdmi"
    help="Turn off when nothing is plugged in. Frees the decoder and some CPU for streaming."
  >
    <Switch id="hdmi" bind:checked={display.hdmi_output} />
  </Field>

  <Field
    label="HDMI ports"
    for="hdmi-ports"
    help="Both ports show the same video and splash screen. Connected screens are detected automatically, including after changing HDMI cables."
    disabled={!display.hdmi_output}
  >
    <Select
      id="hdmi-ports"
      options={PORTS}
      bind:value={display.ports}
      disabled={!display.hdmi_output}
    />
  </Field>

  {#if status?.outputs.length}
    <p class="text-sm text-muted">
      {status.outputs
        .map(
          (output) =>
            `${output.port}: ${output.current ? modeLabel(output.current) : "output off"}`,
        )
        .join(" · ")}
    </p>
  {/if}

  <Field
    label="Resolution"
    for="hdmi-mode"
    help={modeHelp}
    disabled={!display.hdmi_output}
  >
    <Select
      id="hdmi-mode"
      options={modes}
      bind:value={display.mode}
      disabled={!display.hdmi_output}
    />
  </Field>

  <Field label="Scaling" disabled={!display.hdmi_output}>
    <div class="w-full sm:w-64">
      <Segmented
        options={SCALING}
        value={display.scaling}
        disabled={!display.hdmi_output}
        onchange={(v) => (display.scaling = v)}
      />
    </div>
  </Field>

  <Field label="Rotate 180°" for="hdmi-rotate" disabled={!display.hdmi_output}>
    <Switch
      id="hdmi-rotate"
      bind:checked={display.rotate_180}
      disabled={!display.hdmi_output}
    />
  </Field>

  <Field label="Mirror" for="hdmi-mirror" disabled={!display.hdmi_output}>
    <Switch
      id="hdmi-mirror"
      bind:checked={display.mirror}
      disabled={!display.hdmi_output}
    />
  </Field>
</Card>

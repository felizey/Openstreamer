<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { api, message } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import type { InputStatus, UvcDevice, UvcMode } from "@/types/input";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import { btn } from "@/components/ui/buttons";
  import { snackbar } from "@/stores/snackbar.svelte";
  import { developer } from "@/stores/developer.svelte";

  let { input = $bindable() }: { input: Settings["input"] } = $props();

  const SOURCES = [
    { label: "DJI goggles", value: "dji_fpv" },
    { label: "DJI Goggles 3 - USB-A", value: "dji_goggles3" },
    { label: "FPV Goggles V1/V2", value: "dji_fpv_legacy" },
    { label: "USB camera", value: "uvc" },
  ];

  const ABOUT: Record<string, string> = {
    dji_fpv: "DJI Goggles 2, Integra, 3 or N3 connected to the USB-C port.",
    dji_goggles3:
      "DJI Goggles 3 - USB-A. Video only: no camera settings, battery or link quality.",
    dji_fpv_legacy:
      "The first DJI FPV Goggles, V1 or V2, in one of the USB-A ports. Video only: no camera settings, battery or link quality.",
    uvc: "An analog receiver, HDMI capture stick or webcam in one of the USB-A ports. Its video is encoded to H.264 by the Pi's hardware encoder.",
  };

  const FORMAT: Record<string, string> = {
    h264: "H.264",
    mjpeg: "MJPEG",
    raw: "Uncompressed",
  };

  let status = $state<InputStatus | null>(null);
  let devices = $state<UvcDevice[] | null>(null);
  let devicesError = $state<string | null>(null);
  let scanning = $state(false);
  let timer: ReturnType<typeof setInterval> | null = null;

  const goggles3 = $derived(
    status?.mode === "dji_goggles3" ? status.goggles3 : null,
  );
  const goggles3Summary = $derived.by(() => {
    switch (goggles3?.state) {
      case "live":
        return { text: "Receiving video.", tone: "text-ok" };
      case "connecting":
        return {
          text: "DJI Goggles 3 connected. Waiting for video stream. Enable Sharing live view in the goggles.",
          tone: "text-warn",
        };
      case "waiting":
        return {
          text: goggles3?.interface
            ? "DJI Goggles 3 connected. Waiting for video stream. Enable Sharing live view in the goggles."
            : "Connect DJI Goggles 3 to a USB-A port with a data cable.",
          tone: "text-muted",
        };
      case "error":
        return {
          text: `${goggles3.error ?? "Receiver error"}. Retrying…`,
          tone: "text-down",
        };
      default:
        return null;
    }
  });

  const legacy = $derived(
    status?.mode === "dji_fpv_legacy" ? status.legacy : null,
  );
  const legacySummary = $derived.by(() => {
    switch (legacy?.state) {
      case "live":
        return { text: "Receiving video.", tone: "text-ok" };
      case "waiting":
        return {
          text:
            legacy.hint ??
            "Goggles found, no video yet. Power the aircraft and wait for the picture in the goggles.",
          tone: "text-warn",
        };
      case "no_device":
        return {
          text: "No goggles found. Plug them into a USB-A port.",
          tone: "text-muted",
        };
      case "error":
        return { text: `${legacy.error}. Retrying…`, tone: "text-down" };
      default:
        return null;
    }
  });

  const CAPTURE_BYTES = 8 * 1024 * 1024;
  const capturing = $derived(
    legacy?.captured != null && legacy.captured < CAPTURE_BYTES,
  );

  async function capture() {
    try {
      await api.startLegacyCapture();
      await refresh();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  async function refresh() {
    try {
      status = await api.input();
    } catch {
      // Shown as offline elsewhere
    }
  }

  async function scan() {
    scanning = true;
    try {
      devices = await api.inputDevices();
      devicesError = null;
    } catch (e) {
      devicesError = message(e);
    } finally {
      scanning = false;
    }
  }

  onMount(() => {
    refresh();
    scan();
    timer = setInterval(refresh, 3000);
  });
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  // The configured device, or the first one: what "First found" resolves to
  const device = $derived(
    devices?.find((d) => d.path === input.uvc.device) ?? devices?.[0] ?? null,
  );

  const deviceOptions = $derived.by(() => {
    const list = (devices ?? []).map((d) => ({ value: d.path, label: d.name }));
    if (input.uvc.device && !list.some((d) => d.value === input.uvc.device))
      list.unshift({
        value: input.uvc.device,
        label: "Saved device (not plugged in)",
      });
    return [{ value: "", label: "First one found" }, ...list];
  });

  const modeLabel = (m: UvcMode) =>
    `${m.width} × ${m.height} · ${m.fps} fps · ${FORMAT[m.format]}`;

  const modeOptions = $derived.by(() => {
    const list = (device?.modes ?? []).map((m) => ({
      value: m.id,
      label: modeLabel(m),
    }));
    if (input.uvc.mode && !list.some((m) => m.value === input.uvc.mode))
      list.unshift({
        value: input.uvc.mode,
        label: `${input.uvc.mode} (not offered)`,
      });
    return [{ value: "", label: "Best available" }, ...list];
  });

  // Passing the device's H.264 through: nothing to encode, so no bitrate
  const passthrough = $derived(status?.uvc.encoder === "passthrough");

  const ENCODER: Record<string, string> = {
    passthrough: "passed through",
    hardware: "hardware encoding",
    software: "software encoding",
  };

  const summary = $derived.by(() => {
    const u = status?.uvc;
    if (!u || status?.mode !== "uvc") return null;
    switch (u.state) {
      case "live": {
        const mode = u.mode
          ? `${u.mode.width} × ${u.mode.height} · ${u.mode.fps} fps`
          : "";
        const how = [
          u.format && FORMAT[u.format],
          u.encoder && ENCODER[u.encoder],
        ]
          .filter(Boolean)
          .join(", ");
        return { text: `${u.device} · ${mode} · ${how}`, tone: "text-ok" };
      }
      case "starting":
        return {
          text: `Starting ${u.device ?? "the camera"}…`,
          tone: "text-warn",
        };
      case "no_device":
        return {
          text: "No USB camera found. Plug one into a USB-A port.",
          tone: "text-muted",
        };
      case "error":
        return { text: `${u.error}. Retrying…`, tone: "text-down" };
      default:
        return null;
    }
  });
</script>

<div class="flex flex-col gap-4">
  <Card title="Video source">
    <div class="flex flex-col gap-2 px-4 py-3">
      <label class="sr-only" for="video-source">Video source</label>
      <Select id="video-source" options={SOURCES} bind:value={input.mode} />
      <p class="text-[12px] text-muted">
        {ABOUT[input.mode]}
      </p>
      {#if input.mode === "dji_fpv"}
        <p
          class="rounded-xl border border-line bg-warn/10 px-4 py-3 text-[12px] text-fg shadow-sm"
        >
          To turn off OSD in the video feed, turn off "Camera View Recording" in
          Settings → Camera → Advanced Camera Settings
        </p>
      {/if}
    </div>
  </Card>

  {#if input.mode === "dji_goggles3"}
    <Card title="DJI Goggles 3 - USB-A">
      <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
        Experimental: USB needs testing on real Goggles 3 and Raspberry Pi
        hardware. Camera controls and telemetry are unavailable in this mode.
      </p>
      <div class="px-4 py-3 text-[12px] leading-relaxed text-muted">
        <p>
          Connect the goggles to a USB-A port on the Pi with a data cable and
          enable or select "Sharing live view" on the goggles. They must expose
          a DJI USB network device; its interface is found automatically. The
          receiver uses
          <code class="font-mono">192.168.60.2:9003</code>; the Pi uses
          <code class="font-mono">192.168.60.1/24</code> on this link.
        </p>
      </div>
      <Field
        label="Network interface (optional)"
        for="goggles3-interface"
        help="Leave empty to find the DJI USB network interface automatically. Enter its Linux interface name to select it explicitly."
      >
        <TextField
          id="goggles3-interface"
          bind:value={input.goggles3.interface}
          placeholder="Automatic"
          maxlength={15}
          mono
        />
      </Field>
      {#if goggles3Summary}
        <div class="px-4 py-3 text-[12px]" aria-live="polite">
          <p class={goggles3Summary.tone}>{goggles3Summary.text}</p>
          {#if goggles3?.interface || goggles3?.peer}
            <p class="mt-1 font-mono text-[11px] text-muted">
              {[goggles3.interface, goggles3.peer].filter(Boolean).join(" · ")}
            </p>
          {/if}
        </div>
      {/if}
    </Card>
  {/if}

  {#if input.mode === "dji_fpv_legacy"}
    <Card title="FPV Goggles V1/V2">
      <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
        Experimental: not tried on real hardware by maintainers
      </p>
      {#if legacySummary}
        <p class="px-4 py-3 text-[12px] {legacySummary.tone}">
          {legacySummary.text}
        </p>
      {/if}
      {#if developer.enabled}
        <Field
          label="Capture the raw stream"
          help="Saves the next 8 MB exactly as the goggles send them."
        >
          <button
            type="button"
            class={btn.plain}
            disabled={!legacy || capturing}
            onclick={capture}
          >
            {capturing
              ? `Capturing… ${Math.round(((legacy?.captured ?? 0) / CAPTURE_BYTES) * 100)}%`
              : "Capture"}
          </button>
          {#if legacy?.captured}
            <a
              class={btn.text}
              href={api.legacyCaptureUrl}
              download="fpv-goggles-capture.h264"
            >
              Download
            </a>
          {/if}
        </Field>
      {/if}
    </Card>
  {/if}

  {#if input.mode === "uvc"}
    <Card title="USB camera">
      {#snippet aside()}
        <button
          type="button"
          class={btn.plain}
          disabled={scanning}
          onclick={scan}
        >
          {scanning ? "Looking…" : "Refresh"}
        </button>
      {/snippet}

      {#if summary}
        <p class="px-4 py-3 text-[12px] {summary.tone}">{summary.text}</p>
      {/if}
      {#if status?.mode === "uvc" && status.uvc.encoder === "software"}
        <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
          The hardware encoder isn't available, so encoding runs on the CPU, at
          720p at most. Usually that's too little GPU memory: add <code
            class="font-mono">gpu_mem=256</code
          >
          to <code class="font-mono">/boot/firmware/config.txt</code> and reboot.
        </p>
      {/if}
      {#if devicesError}
        <p class="px-4 py-3 text-[12px] text-down">{devicesError}</p>
      {/if}

      <Field label="Device" for="uvc-device">
        <Select
          id="uvc-device"
          options={deviceOptions}
          bind:value={input.uvc.device}
        />
      </Field>

      <Field label="Resolution" for="uvc-mode">
        <Select
          id="uvc-mode"
          options={modeOptions}
          bind:value={input.uvc.mode}
        />
      </Field>

      <Field
        label="Bitrate (kbit/s)"
        for="uvc-bitrate"
        help={passthrough
          ? "This device sends H.264 itself, so its own bitrate applies."
          : "About 6000 for 1080p, 2500 for an analog receiver."}
        disabled={passthrough}
      >
        <TextField
          id="uvc-bitrate"
          type="number"
          min={500}
          max={20000}
          step={500}
          mono
          bind:value={input.uvc.bitrate_kbps}
          disabled={passthrough}
        />
      </Field>

      <Field
        label="Keyframe every (s)"
        for="uvc-keyframe"
        disabled={passthrough}
      >
        <TextField
          id="uvc-keyframe"
          type="number"
          min={1}
          max={10}
          mono
          bind:value={input.uvc.keyframe_secs}
          disabled={passthrough}
        />
      </Field>
    </Card>
  {/if}
</div>
